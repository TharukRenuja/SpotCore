use spotcore::{Backend, BackendFuture, Capabilities, Client, Command, Error};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
    time::Duration,
};

#[derive(Default)]
struct FakeBackend {
    calls: Vec<Command>,
    fail: bool,
}
impl Backend for FakeBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            pause: true,
            ..Default::default()
        }
    }
    fn execute(&mut self, command: Command) -> BackendFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            if !self.capabilities().supports(&command) {
                return Err(Error::Unsupported);
            }
            self.calls.push(command);
            if self.fail {
                Err(Error::Disconnected)
            } else {
                Ok(())
            }
        })
    }
}

// These test futures complete immediately; a pending result is a test failure.
fn ready<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::noop();
    match std::pin::pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(waker))
    {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("expected an immediately ready test future"),
    }
}

#[test]
fn rejects_unsupported_commands_without_dispatch() {
    let mut client = Client::new(FakeBackend::default());
    for command in [
        Command::Load("spotify:track:4uLU6hMCjMI75M1A2tKUQC".into()),
        Command::Stop,
        Command::Resume,
        Command::Seek(Duration::ZERO),
        Command::Next,
        Command::Previous,
    ] {
        assert_eq!(ready(client.execute(command)), Err(Error::Unsupported));
    }
    assert!(client.into_backend().calls.is_empty());
}

#[test]
fn dispatches_supported_commands_and_preserves_errors() {
    let mut client = Client::new(FakeBackend::default());
    assert_eq!(ready(client.execute(Command::Pause)), Ok(()));
    assert_eq!(client.into_backend().calls, vec![Command::Pause]);
    let mut client = Client::new(FakeBackend {
        fail: true,
        ..Default::default()
    });
    assert_eq!(
        ready(client.execute(Command::Pause)),
        Err(Error::Disconnected)
    );
}

#[test]
fn event_api_reports_unsupported_for_command_only_backend() {
    let mut client = Client::new(FakeBackend::default());
    assert_eq!(ready(client.next_event()), Err(Error::Unsupported));
}

#[test]
fn invalid_volume_is_rejected_before_dispatch() {
    let mut client = Client::new(FakeBackend::default());
    for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert_eq!(
            ready(client.set_volume(value)),
            Err(Error::InvalidInput("volume must be 0..=1"))
        );
    }
    assert!(client.into_backend().calls.is_empty());
}
