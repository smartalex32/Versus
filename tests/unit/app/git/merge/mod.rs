use super::*;
use crate::app::git::tests::{GitFixture, merge_request};
use std::{
    fs,
    sync::mpsc,
    time::{Duration, Instant},
};

fn settle_load(window: &mut GitMergeWindow, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while window.loading.is_some() {
        assert!(!window.poll(ctx));
        assert!(Instant::now() < deadline, "Git merge loader did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        window.error.is_none(),
        "Git merge failed to load: {:?}",
        window.error
    );
}

#[test]
fn merge_session_retains_the_exact_partial_result_and_its_utf16_crlf_bom_format() {
    let fixture = GitFixture::new("utf16-result");
    fixture.write("base.txt", "base\r\n");
    fixture.write("local.txt", "current\r\n");
    fixture.write("remote.txt", "incoming\r\n");
    let result = fixture.write(
        "result.txt",
        [
            0xff, 0xfe, b'p', 0, b'a', 0, b'r', 0, b't', 0, b'i', 0, b'a', 0, b'l', 0, b'\r', 0,
            b'\n', 0,
        ],
    );

    let mut session = MergeSession::load(merge_request(&fixture)).unwrap();
    assert_eq!(session.model.result(), "partial\r\n");
    assert_eq!(session.output.text, "partial\r\n");
    assert_eq!(session.output.encoding, versus::TextEncoding::Utf16Le);

    session.model.edit_result("edited\r\n".into()).unwrap();
    let mut candidate = session.save_candidate().unwrap();
    versus::save_working_file(&mut candidate).unwrap();
    let bytes = fs::read(result).unwrap();
    assert_eq!(&bytes[..2], &[0xff, 0xfe]);
    assert!(
        bytes
            .windows(4)
            .any(|window| window == [b'\r', 0, b'\n', 0])
    );
    assert_eq!(
        versus::load_working_file(Some(&candidate.path.clone().unwrap()))
            .unwrap()
            .text,
        "edited\r\n"
    );
}

#[test]
fn missing_base_is_allowed_but_missing_required_sources_are_rejected() {
    let fixture = GitFixture::new("missing-inputs");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    fixture.write("result.txt", "partial\n");
    let mut request = merge_request(&fixture);
    request.base = None;
    assert!(MergeSession::load(request.clone()).is_ok());

    request.local = fixture.path("missing-local.txt");
    let error = MergeSession::load(request).unwrap_err();
    assert!(error.contains("Required Git input does not exist"));
}

#[test]
fn merge_session_refuses_result_aliases_of_any_input() {
    let fixture = GitFixture::new("aliases");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    let mut request = merge_request(&fixture);
    request.output = request.local.clone();

    let error = MergeSession::load(request).unwrap_err();
    assert!(error.contains("Result must have a different destination"));
}

#[test]
fn unresolved_conflicts_block_save_candidates_before_any_write() {
    let fixture = GitFixture::new("blocked-save");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    let result = fixture.write(
        "result.txt",
        "<<<<<<< current\ncurrent\n=======\nincoming\n>>>>>>> incoming\n",
    );
    let session = MergeSession::load(merge_request(&fixture)).unwrap();

    assert!(session.save_candidate().is_err());
    assert!(
        fs::read_to_string(result)
            .unwrap()
            .starts_with("<<<<<<< current")
    );
}

#[test]
fn failed_save_preserves_the_external_destination_and_the_in_memory_result() {
    let fixture = GitFixture::new("external-save");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    let output = fixture.write("result.txt", "partial\n");
    let mut session = MergeSession::load(merge_request(&fixture)).unwrap();
    session
        .model
        .edit_result("edited in Versus\n".into())
        .unwrap();
    let mut candidate = session.save_candidate().unwrap();
    fs::write(&output, "edited outside\n").unwrap();

    assert!(versus::save_working_file(&mut candidate).is_err());
    assert_eq!(fs::read_to_string(output).unwrap(), "edited outside\n");
    assert_eq!(session.model.result(), "edited in Versus\n");
}

#[test]
fn save_and_resolve_reports_success_only_after_the_save_worker_confirms_it() {
    let fixture = GitFixture::new("save-completion");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    fixture.write("result.txt", "partial\n");
    let ctx = egui::Context::default();
    let mut window = GitMergeWindow::new(merge_request(&fixture), &ctx);
    settle_load(&mut window, &ctx);
    let output = window.session.as_ref().unwrap().output.clone();

    let (sender, receiver) = mpsc::channel();
    window.saving = Some(receiver);
    assert!(!window.poll(&ctx));
    assert!(window.is_saving());
    sender.send(Ok(output)).unwrap();
    assert!(window.poll(&ctx));
    assert!(!window.is_saving());
}

#[test]
fn save_and_resolve_writes_the_result_before_signalling_success() {
    let fixture = GitFixture::new("async-success");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    let output = fixture.write("result.txt", "partial\n");
    let ctx = egui::Context::default();
    let mut window = GitMergeWindow::new(merge_request(&fixture), &ctx);
    settle_load(&mut window, &ctx);
    window
        .session
        .as_mut()
        .unwrap()
        .model
        .edit_result("resolved\n".into())
        .unwrap();

    window.start_save(&ctx);
    assert!(window.is_saving());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if window.poll(&ctx) {
            break;
        }
        assert!(Instant::now() < deadline, "Git save worker did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(fs::read_to_string(output).unwrap(), "resolved\n");
}

#[test]
fn failed_async_save_keeps_the_window_model_and_external_destination() {
    let fixture = GitFixture::new("async-failure");
    fixture.write("base.txt", "base\n");
    fixture.write("local.txt", "current\n");
    fixture.write("remote.txt", "incoming\n");
    let output = fixture.write("result.txt", "partial\n");
    let ctx = egui::Context::default();
    let mut window = GitMergeWindow::new(merge_request(&fixture), &ctx);
    settle_load(&mut window, &ctx);
    window
        .session
        .as_mut()
        .unwrap()
        .model
        .edit_result("resolved in window\n".into())
        .unwrap();
    fs::write(&output, "edited outside\n").unwrap();

    window.start_save(&ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while window.is_saving() {
        assert!(!window.poll(&ctx));
        assert!(Instant::now() < deadline, "Git save worker did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        window
            .error
            .as_deref()
            .is_some_and(|error| !error.is_empty())
    );
    assert_eq!(fs::read_to_string(output).unwrap(), "edited outside\n");
    assert_eq!(
        window.session.as_ref().unwrap().model.result(),
        "resolved in window\n"
    );
}
