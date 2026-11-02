use crate::Result;
use std::{
    io::{Read, Seek, SeekFrom},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub struct Output {
    pub success: bool,
    pub text: String,
}
pub fn run(program: &str, args: &[&str]) -> Result<Output> {
    let mut output = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(output.try_clone().map_err(|e| e.to_string())?)
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            other => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{program} did not finish within 20 seconds: {other:?}"
                ));
            }
        }
    };
    output.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(65_536)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(Output {
        success: status.success(),
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}
pub fn checked(program: &str, args: &[&str]) -> Result<String> {
    let output = run(program, args)?;
    if output.success {
        Ok(output.text)
    } else {
        Err(format!("{program} failed: {}", output.text.trim()))
    }
}
