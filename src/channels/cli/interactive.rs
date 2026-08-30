//! Interactive readline loop using reedline.
//!
//! Runs the main REPL: read user input, dispatch meta-commands,
//! forward messages to the channel, and display responses.
//!
//! Implements the "sequential input" model from RFC-014 Phase 2:
//! while a request is being processed, new input is rejected with a
//! message instead of being queued. This prevents the fire-and-forget
//! confusion where responses arrive mid-typing.

use anyhow::Result;
use reedline::{DefaultPrompt, DefaultPromptSegment, Reedline, Signal};

use crate::channels::cli::channel::CliChannelHandle;
use crate::channels::cli::commands::MetaCommand;

/// The interactive read-eval-print loop.
pub struct InteractiveLoop {
    /// Handle to inject messages into the gateway.
    handle: CliChannelHandle,
    /// The reedline line editor.
    editor: Reedline,
    /// The prompt to display.
    prompt: DefaultPrompt,
}

impl InteractiveLoop {
    /// Create a new interactive loop.
    pub fn new(handle: CliChannelHandle) -> Self {
        let editor = Reedline::create();
        let prompt = DefaultPrompt::default();

        Self {
            handle,
            editor,
            prompt,
        }
    }

    /// Create with a custom prompt label.
    #[allow(dead_code)]
    pub fn with_prompt_label(handle: CliChannelHandle, left: &str) -> Self {
        let editor = Reedline::create();
        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(left.to_string()),
            DefaultPromptSegment::Empty,
        );

        Self {
            handle,
            editor,
            prompt,
        }
    }

    /// Run the interactive loop until `.quit` or EOF.
    ///
    /// This is a blocking call. For use inside `tokio::task::spawn_blocking`
    /// or a dedicated thread.
    pub async fn run(&mut self) -> Result<()> {
        println!("Oxios CLI — type .help for commands\n");

        loop {
            let signal = self.editor.read_line(&self.prompt);

            match signal {
                Ok(Signal::Success(line)) => {
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        continue;
                    }

                    // Check for meta-commands.
                    if let Some(cmd) = MetaCommand::parse(&trimmed) {
                        if self.handle_meta(cmd).await? {
                            break; // .quit
                        }
                        continue;
                    }

                    // Reject input while a previous request is still in-flight.
                    if self.handle.is_processing() {
                        println!("⏳ Still processing your previous request. Please wait.");
                        continue;
                    }

                    // Mark as processing, then forward to the gateway.
                    self.handle.set_processing(true);
                    self.handle.send_user_message(trimmed).await?;
                    self.handle.touch_session();

                    // NOTE: The response will arrive asynchronously via the
                    // Channel::send() implementation (printed to stdout).
                    // In a future iteration, we could wait for a response here
                    // for a synchronous feel, but for now the gateway routes
                    // the response back through the channel.
                }
                Ok(Signal::CtrlC) => {
                    println!("\n(Ctrl+C again to quit, or type .quit)");
                }
                Ok(Signal::CtrlD) => {
                    println!("\nGoodbye!");
                    break;
                }
                Err(err) => {
                    tracing::error!("Readline error: {err}");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handle a meta-command. Returns `true` if we should quit.
    async fn handle_meta(&self, cmd: MetaCommand) -> Result<bool> {
        match cmd {
            MetaCommand::Quit => {
                println!("Goodbye!");
                Ok(true)
            }
            MetaCommand::Help => {
                print!("{}", MetaCommand::help_text());
                Ok(false)
            }
            MetaCommand::Reset => {
                self.handle.reset_session();
                println!("Session reset.");
                Ok(false)
            }
            MetaCommand::Model(Some(name)) => {
                println!("Switching model to: {name}");
                self.handle.send_switch_model(&name).await?;
                Ok(false)
            }
            MetaCommand::Model(None) => {
                println!("Current model: (default)");
                Ok(false)
            }
            MetaCommand::Persona(Some(name)) => {
                println!("Switching persona to: {name}");
                self.handle.send_switch_persona(&name).await?;
                Ok(false)
            }
            MetaCommand::Persona(None) => {
                self.handle.send_get_persona().await?;
                Ok(false)
            }
            MetaCommand::Space(None) => {
                // Space info is managed by the kernel via message routing.
                // Channels don't have direct kernel access.
                println!("📋 The .space command is only available in the Web dashboard.");
                Ok(false)
            }
            MetaCommand::Space(Some(_id_or_name)) => {
                // Space switching requires kernel access.
                // Channels don't have direct kernel access.
                println!("📋 The .space command is only available in the Web dashboard.");
                Ok(false)
            }
            MetaCommand::Spaces => {
                println!("📋 The .spaces command is only available in the Web dashboard.");
                Ok(false)
            }
            MetaCommand::Clear => {
                // ANSI clear screen.
                print!("\x1b[2J\x1b[H");
                Ok(false)
            }
        }
    }
}
