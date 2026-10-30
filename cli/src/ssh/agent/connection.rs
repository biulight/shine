//! Transport-independent handshake and authenticated request routing.

use super::transfer::{TransferRequest, handle_transfer};
use super::{BrokerSession, DuplexStream, SessionContext};
use crate::ssh::protocol::{self, ClientMessage, PROTOCOL_VERSION, ServerMessage};
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use zeroize::Zeroize;

pub(super) async fn handle_connection(
    mut stream: impl DuplexStream,
    token: &str,
    context: &SessionContext,
    broker: Option<&BrokerSession>,
) -> Result<()> {
    let hello: ClientMessage = protocol::read_message(&mut stream).await?;
    let ClientMessage::Hello { protocol_version } = hello else {
        bail!("expected a Hello message to open the connection");
    };
    if protocol_version != PROTOCOL_VERSION {
        protocol::write_message(
            &mut stream,
            &ServerMessage::Error {
                message: format!(
                    "protocol version mismatch: local agent speaks v{PROTOCOL_VERSION}, remote shine speaks v{protocol_version}; upgrade whichever side is older"
                ),
            },
        )
        .await?;
        return Ok(());
    }
    protocol::write_message(
        &mut stream,
        &ServerMessage::HelloAck {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .await?;

    let request: ClientMessage = protocol::read_message(&mut stream).await?;
    match request {
        ClientMessage::Hello { .. } => bail!("unexpected second Hello message"),
        ClientMessage::FileBrokerStatus {
            token: request_token,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            protocol::write_message(
                &mut stream,
                &ServerMessage::FileBrokerStatus {
                    enabled: broker.is_some(),
                },
            )
            .await
        }
        ClientMessage::FileRelease {
            token: request_token,
            nonce,
            describe,
            length,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            let Some(broker) = broker else {
                return send_error(&mut stream, "secret broker is not enabled").await;
            };
            if length > crate::ssh::file_broker::MAX_REQUEST {
                return send_error(&mut stream, "file broker request too large").await;
            }
            let _transport = broker.file_transport_guard().await;
            protocol::write_message(&mut stream, &ServerMessage::FileReady).await?;
            let bytes = tokio::time::timeout(
                std::time::Duration::from_secs(300),
                crate::ssh::file_broker::receive_blob(&mut stream, &nonce, 0, length),
            )
            .await
            .map_err(|_| anyhow::anyhow!("file request transfer timed out"))??;
            if bytes.len() != length {
                return send_error(&mut stream, "file broker request length mismatch").await;
            }
            let request: crate::ssh::file_broker::FileReleaseRequest =
                serde_json::from_slice(&bytes)
                    .map_err(|_| anyhow::anyhow!("invalid file release request"))?;
            use tokio::io::AsyncReadExt;
            let mut extra = [0u8; 1];
            let result = tokio::select! {
                result = broker.handle_files(&request, describe, &nonce) => result,
                _ = stream.read(&mut extra) => return send_error(&mut stream, "file broker peer disconnected or sent extra data").await,
            };
            match result {
                Ok(crate::ssh::broker::FileResult::Description(summary)) => {
                    protocol::write_message(
                        &mut stream,
                        &ServerMessage::DescriptionResponse { summary },
                    )
                    .await
                }
                Ok(crate::ssh::broker::FileResult::Plain(values)) => {
                    protocol::write_message(&mut stream, &ServerMessage::FileReady).await?;
                    for (index, bytes) in values.iter().enumerate() {
                        crate::ssh::file_broker::send_blob(&mut stream, &nonce, index, bytes)
                            .await?;
                    }
                    protocol::write_message(
                        &mut stream,
                        &ServerMessage::FileComplete {
                            count: values.len(),
                        },
                    )
                    .await
                }
                Err(error) => send_error(&mut stream, error.to_string()).await,
            }
        }

        ClientMessage::Transfer {
            token: request_token,
            direction,
            remote_spec,
            local_spec,
            force,
            dry_run,
            use_scp,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            handle_transfer(
                &mut stream,
                context,
                TransferRequest {
                    direction,
                    remote_spec,
                    local_spec,
                    force,
                    dry_run,
                    use_scp,
                },
            )
            .await
        }
        ClientMessage::Status {
            token: request_token,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            protocol::write_message(
                &mut stream,
                &ServerMessage::StatusResponse {
                    session_local_dir: context.local_dir.display().to_string(),
                    host: context.host.clone(),
                },
            )
            .await
        }
        ClientMessage::DirectSecret {
            token: request_token,
            specs,
            argv,
            nonce,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            let Some(broker) = broker else {
                return send_error(
                    &mut stream,
                    "secret broker is not enabled for this SSH session",
                )
                .await;
            };
            match broker.handle_direct(&specs, &argv, &nonce).await {
                Ok(values) => send_secret_values(&mut stream, values).await,
                Err(error) => send_error(&mut stream, error.to_string()).await,
            }
        }
        ClientMessage::WorkspaceSecret {
            token: request_token,
            snapshot,
            argv,
            nonce,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            let Some(broker) = broker else {
                return send_error(
                    &mut stream,
                    "secret broker is not enabled for this SSH session",
                )
                .await;
            };
            match broker.handle_workspace(&snapshot, &argv, &nonce).await {
                Ok(values) => send_secret_values(&mut stream, values).await,
                Err(error) => send_error(&mut stream, error.to_string()).await,
            }
        }
        ClientMessage::DescribeWorkspace {
            token: request_token,
            snapshot,
            release,
            argv,
            nonce,
        } => {
            if request_token != token {
                return send_error(&mut stream, "invalid session token").await;
            }
            let Some(broker) = broker else {
                return send_error(
                    &mut stream,
                    "secret broker is not enabled for this SSH session",
                )
                .await;
            };
            match broker
                .handle_description(&snapshot, &release, &argv, &nonce)
                .await
            {
                Ok(summary) => {
                    protocol::write_message(
                        &mut stream,
                        &ServerMessage::DescriptionResponse { summary },
                    )
                    .await
                }
                Err(error) => send_error(&mut stream, error.to_string()).await,
            }
        }
    }
}

async fn send_secret_values(
    stream: &mut impl DuplexStream,
    values: BTreeMap<String, String>,
) -> Result<()> {
    let mut response = ServerMessage::SecretResponse { values };
    let result = protocol::write_message(stream, &response).await;
    if let ServerMessage::SecretResponse { values } = &mut response {
        for value in values.values_mut() {
            value.zeroize();
        }
    }
    result
}

pub(super) async fn send_error(
    stream: &mut impl DuplexStream,
    message: impl Into<String>,
) -> Result<()> {
    protocol::write_message(
        stream,
        &ServerMessage::Error {
            message: message.into(),
        },
    )
    .await
}

#[cfg(test)]
mod tests;
