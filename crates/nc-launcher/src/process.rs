//! Game process lifecycle: spawn without a shell, stream redacted log lines,
//! wait for exit, kill on request.

use std::{process::Stdio, sync::Arc};

use nc_core::{Secret, launch::LaunchCommand, secret::redact};
use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
};

use crate::LauncherError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub stream: &'static str,
    pub level: Option<String>,
    pub text: String,
}

pub type LogFn = Arc<dyn Fn(LogLine) + Send + Sync>;

pub struct RunningGame {
    child: Child,
    pub pid: Option<u32>,
}

/// Windows `CREATE_NO_WINDOW`: no console window for `java.exe`/`javaw.exe`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn spawn(cmd: &LaunchCommand, on_log: LogFn) -> Result<RunningGame, LauncherError> {
    std::fs::create_dir_all(&cmd.cwd).map_err(LauncherError::io(&cmd.cwd))?;
    let mut c = Command::new(&cmd.program);
    c.args(&cmd.args)
        .current_dir(&cmd.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Options injected through the environment would bypass validation.
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .kill_on_drop(false);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);

    tracing::info!(program = %cmd.program.display(), args = ?cmd.redacted_args(), "spawning game");
    let mut child = c.spawn().map_err(|e| LauncherError::Spawn(e.to_string()))?;
    let pid = child.id();
    let secrets: Arc<Vec<Secret>> = Arc::new(cmd.secrets().to_vec());

    if let Some(out) = child.stdout.take() {
        tokio::spawn(pump(out, "stdout", on_log.clone(), secrets.clone()));
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(pump(err, "stderr", on_log, secrets));
    }
    Ok(RunningGame { child, pid })
}

impl RunningGame {
    pub async fn wait(&mut self) -> Result<Option<i32>, LauncherError> {
        let status = self
            .child
            .wait()
            .await
            .map_err(|e| LauncherError::Spawn(e.to_string()))?;
        Ok(status.code())
    }

    pub async fn kill(&mut self) -> Result<(), LauncherError> {
        self.child
            .kill()
            .await
            .map_err(|e| LauncherError::Spawn(e.to_string()))
    }
}

async fn pump(
    reader: impl AsyncRead + Unpin,
    stream: &'static str,
    on_log: LogFn,
    secrets: Arc<Vec<Secret>>,
) {
    let refs: Vec<&Secret> = secrets.iter().collect();
    let mut lines = BufReader::new(reader).lines();
    let mut parser = Log4jXml::default();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(ev) = parser.feed(&line) {
            on_log(LogLine {
                stream,
                level: ev.level,
                text: redact(&ev.text, &refs),
            });
        }
    }
}

/// Minimal parser for the log4j2 `XMLLayout` events that the official
/// logging configuration writes to stdout. Plain lines pass through.
#[derive(Default)]
pub struct Log4jXml {
    buf: Option<String>,
}

pub struct ParsedLine {
    pub level: Option<String>,
    pub text: String,
}

impl Log4jXml {
    pub fn feed(&mut self, line: &str) -> Option<ParsedLine> {
        let trimmed = line.trim_start();
        if self.buf.is_none() && !trimmed.starts_with("<log4j:Event") {
            return Some(ParsedLine {
                level: None,
                text: line.to_owned(),
            });
        }
        let buf = self.buf.get_or_insert_with(String::new);
        buf.push_str(line);
        buf.push('\n');
        if !trimmed.contains("</log4j:Event>") {
            // Guard against unbounded growth from malformed output.
            if buf.len() > 256 * 1024 {
                let text = self.buf.take().unwrap_or_default();
                return Some(ParsedLine { level: None, text });
            }
            return None;
        }
        let event = self.buf.take().unwrap_or_default();
        let level = attr(&event, "level");
        let thread = attr(&event, "thread").unwrap_or_default();
        let logger = attr(&event, "logger").unwrap_or_default();
        let message = between(&event, "<![CDATA[", "]]>").unwrap_or_default();
        let short_logger = logger.rsplit('.').next().unwrap_or(&logger).to_owned();
        Some(ParsedLine {
            text: format!(
                "[{thread}/{}] [{short_logger}]: {message}",
                level.as_deref().unwrap_or("INFO")
            ),
            level,
        })
    }
}

fn attr(s: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = s.find(&needle)? + needle.len();
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_owned())
}

fn between(s: &str, open: &str, close: &str) -> Option<String> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(s[start..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log4j_xml_events_and_passes_plain_lines() {
        let mut p = Log4jXml::default();
        assert_eq!(p.feed("plain line").unwrap().text, "plain line");
        assert!(p.feed(r#"<log4j:Event logger="net.minecraft.client.Minecraft" timestamp="1" level="INFO" thread="Render thread">"#).is_none());
        assert!(
            p.feed("  <log4j:Message><![CDATA[Setting user: Steve]]></log4j:Message>")
                .is_none()
        );
        let ev = p.feed("</log4j:Event>").unwrap();
        assert_eq!(ev.level.as_deref(), Some("INFO"));
        assert_eq!(
            ev.text,
            "[Render thread/INFO] [Minecraft]: Setting user: Steve"
        );
    }
}
