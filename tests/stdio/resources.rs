use super::*;
use mcplease::{Resources, calls, types::*};

struct Catalog;
impl Resources<State> for Catalog {
    async fn list(
        &self,
        state: State,
        _: PaginatedRequestParams,
        context: RequestContext,
    ) -> Result<ListResourcesResult, JsonRpcError> {
        let parked = state.park();
        state.started.send(()).unwrap();
        let _ = parked.recv().await;
        assert!(!context.is_cancelled());
        Ok(ListResourcesResult::default())
    }

    async fn list_templates(
        &self,
        _: State,
        _: PaginatedRequestParams,
        _: RequestContext,
    ) -> Result<ListResourceTemplatesResult, JsonRpcError> {
        Ok(ListResourceTemplatesResult::default())
    }

    async fn read(
        &self,
        state: State,
        params: ReadResourceRequestParams,
        context: RequestContext,
    ) -> Result<ReadResourceResult, JsonRpcError> {
        if params.uri == "park" {
            context.progress(0.5, Some(1.0), Some("reading")).unwrap();
            let parked = state.park();
            state.started.send(()).unwrap();
            let _ = parked.recv().await;
        }
        Ok(ReadResourceResult::default())
    }
}
fn start(config: ServerConfig) -> Server {
    let (client, ServerEnd { input, output }) = pipe::pair();
    let (started_tx, started) = mpsc::channel();
    let waiters = Arc::<Mutex<VecDeque<Release<()>>>>::default();
    let state = State {
        started: started_tx,
        waiters: Arc::clone(&waiters),
        log: Arc::default(),
    };
    let mut calls = calls::<Tools, _>(input, output, state, &config.with_resources(Catalog));
    let notifier = calls.notifier();
    let handle = thread::spawn(move || {
        let executor = Executor::new();
        block_on(executor.run(async {
            while let Some(call) = calls.next().await {
                executor.spawn(call?).detach();
            }
            Ok(())
        }))
    });
    Server {
        client,
        started,
        waiters,
        notifier,
        handle,
    }
}
#[test]
fn discovery_does_not_block_ping_or_tools_and_can_be_cancelled() {
    let mut server = start(config());
    server.client.send(&request(1, "resources/list", json!({})));
    server.wait_started();
    server.client.send(&request(2, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 2);
    server
        .client
        .send(&call(3, "note", json!({"text":"while listing"})));
    assert_eq!(server.client.next()["id"], 3);
    server.client.send(&cancel(1, "stop discovery"));
    server.client.send(&request(4, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 4);
    server.release();
    server
        .client
        .send(&request(5, "resources/templates/list", json!({})));
    let templates = server.client.next();
    assert_eq!(templates["id"], 5);
    assert_eq!(templates["result"]["resourceTemplates"], json!([]));
    server.join().unwrap();
}
#[test]
fn reads_report_progress_share_capacity_and_release_on_cancellation() {
    let mut server = start(config().with_max_concurrent_calls(1));
    server.client.send(&request(
        1,
        "resources/read",
        json!({"uri":"park", "_meta":{"progressToken":"read-1"}}),
    ));
    server.wait_started();
    let progress = server.client.next();
    assert_eq!(progress["method"], "notifications/progress");
    assert_eq!(progress["params"]["progressToken"], "read-1");
    server.client.send(&call(2, "note", json!({"text":"busy"})));
    assert_eq!(
        server.client.next()["error"]["code"],
        error_codes::SERVER_BUSY
    );
    server.client.send(&cancel(1, "stop read"));
    server.client.send(&request(3, "ping", json!({})));
    assert_eq!(server.client.next()["id"], 3);

    // The cancelled read is dropped at its await, which frees its slot; the
    // executor may not have run that drop yet, so probe until admitted.
    let mut probe = 0;
    let read = loop {
        server.client.send(&request(
            format!("read {probe}"),
            "resources/read",
            json!({"uri":"now"}),
        ));
        let response = server.client.next();
        if response.get("error").is_none() {
            break response;
        }
        assert_eq!(response["error"]["code"], error_codes::SERVER_BUSY);
        assert!(probe < 200, "the cancelled read never released its slot");
        probe += 1;
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(read["result"]["contents"], json!([]));
    server.join().unwrap();
}
