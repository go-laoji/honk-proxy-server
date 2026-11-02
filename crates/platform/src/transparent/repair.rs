//! Repair under the original namespace/scope identity while capture is suspended.
use super::{Capture, error, inspection, policy_snapshot, routes};
use aya::programs::{
    CgroupAttachMode, CgroupSock, CgroupSockAddr, LinkOrder, SchedClassifier, tc::TcAttachOptions,
};
use hps_ebpf_abi::{TP_ENABLED, TP_SUSPENDED};

impl Capture {
    /// Does not resume forwarding. The caller must drain old owners and pending
    /// metadata and register a new receiver before publishing an enabled policy.
    pub fn reconcile(&self) -> Result<(), String> {
        let mut guard = self.state.lock().unwrap();
        let state = guard.as_mut().ok_or("transparent capture is shut down")?;
        let policy = policy_snapshot(state)?;
        if policy.flags & (TP_ENABLED | TP_SUSPENDED) != TP_ENABLED | TP_SUSPENDED {
            return Err("capture repair requires a suspended enabled policy".into());
        }
        if !inspection::identity_present(self, state)? {
            return Err(
                "capture scope identity changed; explicit reconfiguration is required".into(),
            );
        }
        if let Some(cgroup) = &state.cgroup {
            for attachment in &self.cgroup_attachments {
                if inspection::cgroup_contains(cgroup, attachment)? {
                    continue;
                }
                let mut owned = attachment.link.lock().unwrap();
                match owned.take() {
                    Some(inspection::CgroupLink::Address(link)) => {
                        let program: &mut CgroupSockAddr = state
                            .bpf
                            .program_mut(attachment.name)
                            .ok_or("capture address program missing")?
                            .try_into()
                            .map_err(|e| error("repair address program", e))?;
                        program
                            .detach(link)
                            .map_err(|e| error("release old cgroup link", e))?;
                    }
                    Some(inspection::CgroupLink::Socket(link)) => {
                        let program: &mut CgroupSock = state
                            .bpf
                            .program_mut(attachment.name)
                            .ok_or("capture socket program missing")?
                            .try_into()
                            .map_err(|e| error("repair socket program", e))?;
                        program
                            .detach(link)
                            .map_err(|e| error("release old socket link", e))?;
                    }
                    None => {}
                }
                *owned = Some(
                    if attachment.attach_type >= 10 && attachment.attach_type <= 15 {
                        let program: &mut CgroupSockAddr = state
                            .bpf
                            .program_mut(attachment.name)
                            .ok_or("capture address program missing")?
                            .try_into()
                            .map_err(|e| error("repair address program", e))?;
                        inspection::CgroupLink::Address(
                            program
                                .attach(cgroup, CgroupAttachMode::Single)
                                .map_err(|e| error("repair cgroup link", e))?,
                        )
                    } else {
                        let program: &mut CgroupSock = state
                            .bpf
                            .program_mut(attachment.name)
                            .ok_or("capture socket program missing")?
                            .try_into()
                            .map_err(|e| error("repair socket program", e))?;
                        inspection::CgroupLink::Socket(
                            program
                                .attach(cgroup, CgroupAttachMode::Single)
                                .map_err(|e| error("repair socket link", e))?,
                        )
                    },
                );
            }
        }
        for attachment in &self.attachments {
            let (_, programs) =
                SchedClassifier::query_tcx(&attachment.interface, attachment.direction)
                    .map_err(|e| error("repair TCX inspection", e))?;
            if programs.iter().any(|p| p.id() == attachment.id) {
                continue;
            }
            let program: &mut SchedClassifier = state
                .bpf
                .program_mut(
                    if attachment.direction == aya::programs::TcAttachType::Egress {
                        "hps_tp_egress"
                    } else {
                        "hps_tp_ingress"
                    },
                )
                .ok_or("capture TCX program missing")?
                .try_into()
                .map_err(|e| error("repair TCX program", e))?;
            let mut owned = attachment.link.lock().unwrap();
            if let Some(link) = owned.take() {
                program
                    .detach(link)
                    .map_err(|e| error("release old TCX link", e))?;
            }
            *owned = Some(
                program
                    .attach_with_options(
                        &attachment.interface,
                        attachment.direction,
                        TcAttachOptions::TcxOrder(LinkOrder::last()),
                    )
                    .map_err(|e| error("repair TCX link", e))?,
            );
        }
        if !state
            .routes
            .as_ref()
            .map(|r| r.present())
            .transpose()?
            .unwrap_or(false)
        {
            if let Some(routes) = &mut state.routes {
                routes.cleanup()?;
            }
            state.routes = None;
            state.routes = Some(routes::OwnedRoutes::prepare(
                &self.specification,
                state.policy.netns_cookie,
                &state.ownership,
            )?);
        }
        Ok(())
    }
}
