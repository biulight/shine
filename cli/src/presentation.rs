//! CLI-only lifecycle presentation events and terminal rendering.
//!
//! Lifecycle executors emit these events instead of writing to stdout/stderr
//! directly. The events are intentionally not serializable and may contain
//! human-facing paths or errors that must never enter `LifecycleResultV1`.

use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;

use crate::output::{SectionPrefix, SectionSeparator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentationStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PresentationEvent {
    Line {
        stream: PresentationStream,
        text: String,
    },
    BlankLine,
    SectionStart,
}

impl PresentationEvent {
    pub(crate) fn stdout(text: impl Into<String>) -> Self {
        Self::Line {
            stream: PresentationStream::Stdout,
            text: text.into(),
        }
    }

    pub(crate) fn stderr(text: impl Into<String>) -> Self {
        Self::Line {
            stream: PresentationStream::Stderr,
            text: text.into(),
        }
    }
}

pub(crate) trait LifecycleReporter {
    fn emit(&mut self, event: PresentationEvent);
}

pub(crate) trait LifecycleInteraction {
    fn confirm(&mut self, prompt: &str, default: bool) -> anyhow::Result<bool>;

    #[allow(dead_code)]
    fn authorize_admin<'a>(
        &'a mut self,
        item_count: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + 'a>>;
}

#[derive(Default)]
pub(crate) struct TerminalInteraction;

impl LifecycleInteraction for TerminalInteraction {
    fn confirm(&mut self, prompt: &str, default: bool) -> anyhow::Result<bool> {
        Ok(dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(default)
            .interact()?)
    }

    fn authorize_admin<'a>(
        &'a mut self,
        item_count: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + 'a>> {
        Box::pin(crate::privilege::ensure_admin(item_count))
    }
}

impl shine_core::runtime::RuntimeInteraction for TerminalInteraction {
    fn confirm(&mut self, code: &'static str, default: bool) -> anyhow::Result<bool> {
        LifecycleInteraction::confirm(self, code, default)
    }

    fn authorize_admin<'a>(
        &'a mut self,
        item_count: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(crate::privilege::ensure_admin(item_count))
    }

    fn select_many(
        &mut self,
        code: &'static str,
        choices: &[String],
        defaults: &[String],
    ) -> anyhow::Result<Vec<String>> {
        let selected = dialoguer::MultiSelect::new()
            .with_prompt(code)
            .items(choices)
            .defaults(
                &choices
                    .iter()
                    .map(|choice| defaults.contains(choice))
                    .collect::<Vec<_>>(),
            )
            .interact()?;
        Ok(selected
            .into_iter()
            .map(|index| choices[index].clone())
            .collect())
    }
}

pub(crate) struct TerminalRenderer<'a, O: Write, E: Write> {
    stdout: O,
    stderr: E,
    separator: Option<&'a mut SectionSeparator>,
}

impl<'a, O: Write, E: Write> TerminalRenderer<'a, O, E> {
    pub(crate) fn new(stdout: O, stderr: E, separator: Option<&'a mut SectionSeparator>) -> Self {
        Self {
            stdout,
            stderr,
            separator,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_writers(self) -> (O, E) {
        (self.stdout, self.stderr)
    }

    fn write_stdout_line(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.stdout, "{text}")
    }
}

impl TerminalRenderer<'static, io::Stdout, io::Stderr> {
    pub(crate) fn stdio() -> Self {
        Self::new(io::stdout(), io::stderr(), None)
    }
}

impl<'a> TerminalRenderer<'a, io::Stdout, io::Stderr> {
    pub(crate) fn stdio_with_separator(separator: &'a mut SectionSeparator) -> Self {
        Self::new(io::stdout(), io::stderr(), Some(separator))
    }
}

impl<O: Write, E: Write> LifecycleReporter for TerminalRenderer<'_, O, E> {
    fn emit(&mut self, event: PresentationEvent) {
        let result = match event {
            PresentationEvent::Line { stream, text } => match stream {
                PresentationStream::Stdout => writeln!(self.stdout, "{text}"),
                PresentationStream::Stderr => writeln!(self.stderr, "{text}"),
            },
            PresentationEvent::BlankLine => self.write_stdout_line(""),
            PresentationEvent::SectionStart => {
                let prefix = self
                    .separator
                    .as_deref_mut()
                    .map(SectionSeparator::next_prefix)
                    .unwrap_or(SectionPrefix::None);
                match prefix {
                    SectionPrefix::None => Ok(()),
                    SectionPrefix::BlankLine => self.write_stdout_line(""),
                    SectionPrefix::Preamble(text) => self.write_stdout_line(&text),
                }
            }
        };
        result.expect("writing lifecycle presentation");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingReporter {
        events: Vec<PresentationEvent>,
    }

    impl LifecycleReporter for RecordingReporter {
        fn emit(&mut self, event: PresentationEvent) {
            self.events.push(event);
        }
    }

    struct FakeInteraction {
        confirm: bool,
        authorize: bool,
    }

    impl LifecycleInteraction for FakeInteraction {
        fn confirm(&mut self, _prompt: &str, _default: bool) -> anyhow::Result<bool> {
            Ok(self.confirm)
        }

        fn authorize_admin<'a>(
            &'a mut self,
            _item_count: usize,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + 'a>> {
            Box::pin(async move { Ok(self.authorize) })
        }
    }

    #[test]
    fn writer_backed_renderer_preserves_streams_and_sections() {
        let mut separator = SectionSeparator::with_preamble("Upgrading configs");
        let mut renderer = TerminalRenderer::new(Vec::new(), Vec::new(), Some(&mut separator));
        renderer.emit(PresentationEvent::SectionStart);
        renderer.emit(PresentationEvent::stdout("App Configs"));
        renderer.emit(PresentationEvent::stderr("warning"));
        renderer.emit(PresentationEvent::SectionStart);
        renderer.emit(PresentationEvent::stdout("Shell Presets"));
        let (stdout, stderr) = renderer.into_writers();

        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "Upgrading configs\nApp Configs\n\nShell Presets\n"
        );
        assert_eq!(String::from_utf8(stderr).unwrap(), "warning\n");
    }

    #[tokio::test]
    async fn presentation_and_interaction_ports_are_replaceable() {
        let mut reporter = RecordingReporter::default();
        reporter.emit(PresentationEvent::stdout("changed"));
        reporter.emit(PresentationEvent::stderr("failed"));
        assert_eq!(
            reporter.events,
            vec![
                PresentationEvent::stdout("changed"),
                PresentationEvent::stderr("failed")
            ]
        );

        let mut interaction = FakeInteraction {
            confirm: false,
            authorize: true,
        };
        assert!(!interaction.confirm("remove?", false).unwrap());
        assert!(interaction.authorize_admin(2).await.unwrap());
    }
}
