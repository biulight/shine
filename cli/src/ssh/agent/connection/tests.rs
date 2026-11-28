use super::*;
use crate::env::broker::WorkspaceSnapshot;

fn context() -> SessionContext {
    SessionContext {
        host: "dev".into(),
        ssh_options: vec![],
        local_dir: std::env::temp_dir(),
        control_path: None,
    }
}

fn snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        workspace_path: "/remote/shine.workspace.toml".into(),
        workspace_contents: String::new(),
        mode: "dev".into(),
        override_process_env: false,
        sources: vec![],
    }
}

const HELLO: ClientMessage = ClientMessage::Hello {
    protocol_version: PROTOCOL_VERSION,
};

async fn handshake(peer: &mut tokio::io::DuplexStream) {
    protocol::write_message(peer, &HELLO).await.unwrap();
    let ack: ServerMessage = protocol::read_message(peer).await.unwrap();
    assert!(matches!(
        ack,
        ServerMessage::HelloAck {
            protocol_version: PROTOCOL_VERSION
        }
    ));
}

async fn request_error(message: ClientMessage, broker: Option<&BrokerSession>) -> String {
    let (agent, mut peer) = tokio::io::duplex(8192);
    let context = context();
    let server = handle_connection(agent, "session", &context, broker);
    let client = async {
        handshake(&mut peer).await;
        protocol::write_message(&mut peer, &message).await.unwrap();
        protocol::read_message(&mut peer).await.unwrap()
    };
    let (result, response) = tokio::join!(server, client);
    result.unwrap();
    let ServerMessage::Error { message } = response else {
        panic!("expected request to be rejected")
    };
    message
}

#[tokio::test]
async fn every_request_checks_token_before_transfer_status_or_broker_dispatch() {
    for (token, expected) in [
        ("wrong", "invalid session token"),
        (
            "session",
            "secret broker is not enabled for this SSH session",
        ),
    ] {
        let messages = [
            ClientMessage::DirectSecret {
                token: token.into(),
                specs: vec!["TOKEN".into()],
                argv: vec!["tool".into()],
                nonce: "nonce-for-routing-test".into(),
            },
            ClientMessage::WorkspaceSecret {
                token: token.into(),
                snapshot: snapshot(),
                argv: vec![],
                nonce: "nonce-for-routing-test".into(),
            },
            ClientMessage::DescribeWorkspace {
                token: token.into(),
                snapshot: snapshot(),
                release: vec![],
                argv: vec![],
                nonce: "nonce-for-routing-test".into(),
            },
        ];
        for message in messages {
            assert_eq!(request_error(message, None).await, expected);
        }
    }
    for message in [
        ClientMessage::Status {
            token: "wrong".into(),
        },
        ClientMessage::Transfer {
            token: "wrong".into(),
            direction: crate::ssh::protocol::Direction::Upload,
            remote_spec: "-oProxyCommand=evil".into(),
            local_spec: None,
            force: true,
            dry_run: false,
            use_scp: false,
        },
    ] {
        assert_eq!(request_error(message, None).await, "invalid session token");
    }
}

#[tokio::test]
async fn handshake_rejects_wrong_version_missing_hello_and_second_hello() {
    for hello in [
        ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
        },
        ClientMessage::Status {
            token: "session".into(),
        },
    ] {
        let wrong_version = matches!(hello, ClientMessage::Hello { .. });
        let (agent, mut peer) = tokio::io::duplex(8192);
        let context = context();
        let server = handle_connection(agent, "session", &context, None);
        let client = async {
            protocol::write_message(&mut peer, &hello).await.unwrap();
            protocol::read_message::<_, ServerMessage>(&mut peer).await
        };
        let (result, response) = tokio::join!(server, client);
        if wrong_version {
            result.unwrap();
            assert!(
                matches!(response.unwrap(), ServerMessage::Error { message } if message.contains("protocol version mismatch"))
            );
        } else {
            assert!(result.unwrap_err().to_string().contains("expected a Hello"));
            assert!(response.is_err());
        }
    }
    let (agent, mut peer) = tokio::io::duplex(8192);
    let context = context();
    let server = handle_connection(agent, "session", &context, None);
    let client = async {
        handshake(&mut peer).await;
        protocol::write_message(&mut peer, &HELLO).await.unwrap();
    };
    let (result, ()) = tokio::join!(server, client);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unexpected second Hello")
    );
}

#[tokio::test]
async fn enabled_broker_retains_exact_authorization_and_nonce_replay_rejection() {
    let dir = crate::test_support::make_temp_dir("shine-agent-broker-routing").await;
    let config = crate::config::Config::new_for_test(&dir);
    let broker = BrokerSession::prepare(
        &config,
        "dev",
        true,
        &[],
        &[],
        false,
        false,
        false,
        false,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let message = |token: &str| ClientMessage::DirectSecret {
        token: token.into(),
        specs: vec!["TOKEN".into()],
        argv: vec!["tool".into()],
        nonce: "nonce-authorization-test".into(),
    };
    for (token, expected) in [
        ("wrong", "invalid session token"),
        ("session", "not authorized for this SSH session"),
        ("session", "nonce has already been used"),
    ] {
        let error = request_error(message(token), Some(&broker)).await;
        assert!(
            error.contains(expected),
            "expected {expected:?}, got {error:?}"
        );
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
