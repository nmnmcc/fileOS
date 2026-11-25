// SPDX-License-Identifier: AGPL-3.0-only

//! Isolate image decoding in a short-lived, bounded worker process.

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Semaphore;

const MAX_ACTIVE: usize = 2;
const MAX_OUTPUT: u64 = 8 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct Preview {
    pub(crate) executable: Box<Path>,
    permits: Semaphore,
}

impl Preview {
    pub(crate) fn new(executable: Box<Path>) -> Self {
        Self {
            executable,
            permits: Semaphore::new(MAX_ACTIVE),
        }
    }

    pub(crate) async fn run(
        &self,
        bytes: &[u8],
        side: u32,
    ) -> io::Result<Option<(Vec<u8>, &'static str)>> {
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "thumbnail workers are busy"))?;
        tokio::time::timeout(TIMEOUT, self.decode(bytes, side)).await?
    }

    async fn decode(&self, bytes: &[u8], side: u32) -> io::Result<Option<(Vec<u8>, &'static str)>> {
        let mut child = Command::new(self.executable.as_os_str())
            .args(["internal", "thumbnail-worker", &side.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("missing worker stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing worker stdout"))?;
        let write = async move {
            match stdin.write_all(bytes).await {
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
                result => result,
            }
        };
        let read = async move {
            let mut output = Vec::new();
            stdout.take(MAX_OUTPUT + 1).read_to_end(&mut output).await?;
            if output.len() as u64 > MAX_OUTPUT {
                return Err(io::Error::other(
                    "thumbnail worker exceeded the output limit",
                ));
            }
            Ok(output)
        };
        let ((), mut output) = tokio::try_join!(write, read)?;
        let status = child.wait().await?;
        if status.code() == Some(2) {
            return Ok(None);
        }
        if !status.success() {
            return Err(io::Error::other(format!(
                "thumbnail worker exited with {status}"
            )));
        }
        let media = match output.first() {
            Some(b'j') => "image/jpeg",
            Some(b'p') => "image/png",
            _ => return Err(io::Error::other("invalid thumbnail worker output")),
        };
        output.remove(0);
        Ok(Some((output, media)))
    }
}
