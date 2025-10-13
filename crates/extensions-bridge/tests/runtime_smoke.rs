use extensions_bridge::{Bridge, BridgeEvent, BridgeRequest, ExtensionId, ExtensionsBridge, Scope};
use tokio::sync::broadcast;
use uuid::Uuid;

#[tokio::test]
async fn enable_open_invoke_flow() {
    let (bus, mut rx) = broadcast::channel(8);
    let ext = ExtensionId("ext.sample".into());
    let bridge = ExtensionsBridge::new(bus.clone(), vec![ext.clone()]);

    bridge.enable_bridge().await.unwrap();
    match rx.recv().await.unwrap() {
        BridgeEvent::BridgeReady { extensions } => {
            assert_eq!(extensions, vec![ext.clone()]);
        }
        other => panic!("unexpected event: {:?}", other),
    }

    let channel_id = bridge
        .open_channel(ext.clone(), Scope::Tab)
        .await
        .expect("open channel");
    match rx.recv().await.unwrap() {
        BridgeEvent::ChannelOpen {
            extension,
            scope,
            channel,
        } => {
            assert_eq!(extension, ext);
            assert_eq!(scope as u8, Scope::Tab as u8);
            assert_eq!(channel, channel_id);
        }
        other => panic!("unexpected event: {:?}", other),
    }

    let request = BridgeRequest {
        req_id: Uuid::new_v4(),
        op: "wallet.sign".into(),
        payload: serde_json::json!({"data": "payload"}),
        deadline_ms: 5000,
    };

    let response = bridge
        .invoke(ext.clone(), Scope::Tab, request)
        .await
        .expect("invoke through bridge");

    assert!(response.ok);
    assert!(response.error.is_none());

    match rx.recv().await.unwrap() {
        BridgeEvent::InvokeOk { extension, op } => {
            assert_eq!(extension, ext);
            assert_eq!(op, "wallet.sign");
        }
        other => panic!("unexpected event: {:?}", other),
    }

    bridge.disable_bridge().await.unwrap();
    // There should be a ChannelClosed event for the previously opened channel.
    match rx.recv().await.unwrap() {
        BridgeEvent::ChannelClosed {
            extension,
            scope,
            channel,
        } => {
            assert_eq!(extension, ext);
            assert_eq!(scope as u8, Scope::Tab as u8);
            assert_eq!(channel, channel_id);
        }
        other => panic!("unexpected event: {:?}", other),
    }
}
