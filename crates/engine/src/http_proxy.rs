use crate::{
    BoxIo, ProxyError,
    runtime::Session,
    target::Target,
    traffic::{ActiveIo, Budget, relay},
};
use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, Request, Response, StatusCode, Uri, header};
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type Body = UnsyncBoxBody<Bytes, BoxError>;

pub(crate) async fn serve(io: BoxIo, session: Session) -> Result<(), ProxyError> {
    let service_session = session.clone();
    let service = service_fn(move |request| {
        let session = service_session.clone();
        async move { Ok::<_, Infallible>(handle(request, session).await) }
    });
    hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(session.handshake_timeout())
        .max_buf_size(session.limits.max_header_bytes)
        .max_headers(128)
        .half_close(true)
        .serve_connection(TokioIo::new(io), service)
        .with_upgrades()
        .await
        .map_err(|_| ProxyError::Protocol)
}

fn error(status: StatusCode, message: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(
            Full::new(Bytes::from_static(message.as_bytes()))
                .map_err(|e: Infallible| match e {})
                .boxed_unsync(),
        )
        .unwrap()
}
fn gateway(error_kind: ProxyError) -> Response<Body> {
    match error_kind {
        ProxyError::NoUpstream => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "No eligible upstream; direct fallback is disabled\n",
        ),
        ProxyError::Timeout => error(
            StatusCode::GATEWAY_TIMEOUT,
            "Upstream handshake timed out\n",
        ),
        _ => error(
            StatusCode::BAD_GATEWAY,
            "Upstream connection or response failed\n",
        ),
    }
}

async fn handle(mut request: Request<Incoming>, session: Session) -> Response<Body> {
    if request.method() == Method::CONNECT {
        let target = match Target::from_authority(&request.uri().to_string()) {
            Ok(target) => target,
            Err(_) => return error(StatusCode::BAD_REQUEST, "CONNECT requires host:port\n"),
        };
        let connected = match session
            .dialer
            .connect_context(&target, false, &session.selection_context)
            .await
        {
            Ok(connected) => connected,
            Err(e) => {
                session.failed();
                return gateway(e);
            }
        };
        let upgrade = hyper::upgrade::on(&mut request);
        let child = session.clone();
        session.spawn_io(async move {
            if let Ok(upgraded) = upgrade.await {
                let left: BoxIo = Box::new(ActiveIo::new(
                    TokioIo::new(upgraded),
                    child.activity.clone(),
                ));
                let right: BoxIo = Box::new(ActiveIo::new(connected.io, child.activity.clone()));
                if relay(left, right, child.limits.buffer_bytes, child.budget.clone())
                    .await
                    .is_err()
                {
                    child.failed();
                }
            }
        });
        return Response::new(
            Full::new(Bytes::new())
                .map_err(|e: Infallible| match e {})
                .boxed_unsync(),
        );
    }
    if request.headers().contains_key(header::UPGRADE) {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "HTTP Upgrade is not supported by this version\n",
        );
    }
    if request.uri().scheme_str() != Some("http") {
        return error(
            StatusCode::BAD_REQUEST,
            "Use an absolute http URI or CONNECT for HTTPS targets\n",
        );
    }
    let Some(authority) = request.uri().authority() else {
        return error(StatusCode::BAD_REQUEST, "Missing destination\n");
    };
    if authority.as_str().contains('@') {
        return error(StatusCode::BAD_REQUEST, "Invalid destination authority\n");
    }
    // Authority::port() also returns None for an invalid/out-of-range port;
    // distinguish an omitted port from a malformed explicit one.
    let suffix = authority
        .as_str()
        .strip_prefix(authority.host())
        .unwrap_or("invalid");
    let port = if suffix.is_empty() {
        Some(80)
    } else {
        suffix
            .strip_prefix(':')
            .and_then(|value| value.parse::<u16>().ok())
    };
    let Some(port) = port else {
        return error(StatusCode::BAD_REQUEST, "Invalid destination port\n");
    };
    let target = match Target::new(authority.host(), port) {
        Ok(target) => target,
        Err(_) => return error(StatusCode::BAD_REQUEST, "Invalid destination\n"),
    };
    let connected = match session
        .dialer
        .connect_context(&target, true, &session.selection_context)
        .await
    {
        Ok(connected) => connected,
        Err(e) => {
            session.failed();
            return gateway(e);
        }
    };
    strip_hop(request.headers_mut());
    request.headers_mut().insert(
        header::HOST,
        HeaderValue::from_str(&target.authority()).expect("validated authority"),
    );
    request
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    if connected.absolute_form {
        if let Some(auth) = connected.proxy_authorization {
            request
                .headers_mut()
                .insert(header::PROXY_AUTHORIZATION, auth);
        }
    } else {
        let uri: Uri = request
            .uri()
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/")
            .parse()
            .expect("valid path");
        *request.uri_mut() = uri;
    }
    *request.version_mut() = http::Version::HTTP_11;
    let upstream_id = connected.upstream_id;
    let identity = connected.identity;
    let absolute_form = connected.absolute_form;
    let io = ActiveIo::new(connected.io, session.activity.clone());
    let (mut sender, connection) = match hyper::client::conn::http1::Builder::new()
        .max_buf_size(session.limits.max_header_bytes)
        .handshake(TokioIo::new(io))
        .await
    {
        Ok(connection) => connection,
        Err(_) => {
            session.failed();
            return gateway(ProxyError::Upstream);
        }
    };
    let exchange = session.cancel.child_token();
    let cancelled = exchange.clone();
    session.tracker.spawn(async move {
        tokio::select! { biased; _=cancelled.cancelled()=>{}, _=connection=>{} }
    });
    let request = request.map(|incoming| {
        limited(
            incoming,
            session.budget.clone(),
            session.limits.buffer_bytes,
            None,
        )
    });
    let response = sender.send_request(request).await;
    if response.is_err() {
        session.dialer.traffic_feedback(
            &upstream_id,
            &identity,
            hps_domain::ProbePhase::TargetHttp,
            "upstream_response_failed",
            false,
        );
    }
    if let Ok(response) = &response {
        let auth = absolute_form && response.status() == StatusCode::PROXY_AUTHENTICATION_REQUIRED;
        session.dialer.traffic_feedback(
            &upstream_id,
            &identity,
            if auth {
                hps_domain::ProbePhase::Authentication
            } else {
                hps_domain::ProbePhase::TargetHttp
            },
            if auth {
                "upstream_authentication_failed"
            } else if matches!(response.status().as_u16(), 403 | 429) {
                "target_rejected"
            } else {
                "target_response"
            },
            auth,
        );
    }
    let mut response = match response {
        Ok(response)
            if response.status() != StatusCode::PROXY_AUTHENTICATION_REQUIRED
                && response.status() != StatusCode::SWITCHING_PROTOCOLS =>
        {
            response
        }
        _ => {
            exchange.cancel();
            session.failed();
            return gateway(ProxyError::Upstream);
        }
    };
    strip_hop(response.headers_mut());
    response.map(|incoming| {
        limited(
            incoming,
            session.budget,
            session.limits.buffer_bytes,
            Some(exchange.drop_guard()),
        )
    })
}

fn strip_hop(headers: &mut HeaderMap) {
    let names: Vec<_> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|v| http::HeaderName::from_bytes(v.trim().as_bytes()).ok())
        .collect();
    for name in names {
        headers.remove(name);
    }
    for name in [
        "connection",
        "proxy-connection",
        "keep-alive",
        "transfer-encoding",
        "te",
        "trailer",
        "upgrade",
        "proxy-authorization",
        "proxy-authenticate",
    ] {
        headers.remove(name);
    }
}
fn limited(
    mut body: Incoming,
    budget: Budget,
    chunk_size: usize,
    guard: Option<tokio_util::sync::DropGuard>,
) -> Body {
    let stream = async_stream::try_stream! {
        let _guard=guard;
        while let Some(frame)=body.frame().await {
            let frame=frame.map_err(|error|Box::new(error) as BoxError)?;
            match frame.into_data() {
                Ok(mut bytes)=>{
                    while !bytes.is_empty() {
                        let bytes=bytes.split_to(bytes.len().min(chunk_size));
                        budget.wait(bytes.len()).await;
                        yield hyper::body::Frame::data(bytes);
                    }
                }
                Err(frame)=>yield frame,
            }
        }
    };
    StreamBody::new(stream).boxed_unsync()
}
