use anyhow::Error;
use std::sync::Arc;
use tokio::{
    io::{stdout, AsyncWriteExt},
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        Mutex,
    },
    task::{spawn, JoinHandle},
};

#[derive(Clone, Debug)]
pub struct StdoutChannel {
    receiver: Arc<Mutex<UnboundedReceiver<Option<String>>>>,
    sender: Arc<UnboundedSender<Option<String>>>,
}

impl Default for StdoutChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl StdoutChannel {
    pub fn new() -> Self {
        let (sender, receiver) = unbounded_channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let sender = Arc::new(sender);
        Self { receiver, sender }
    }

    pub fn send(&self, item: String) -> Result<(), Error> {
        self.sender.send(Some(item)).map_err(Into::into)
    }

    async fn recv(&self) -> Option<Option<String>> {
        self.receiver.lock().await.recv().await
    }

    pub async fn close(&self) -> Result<(), Error> {
        self.sender.send(None).map_err(Into::into)
    }

    async fn stdout_task(&self) -> Result<(), Error> {
        while let Some(Some(line)) = self.recv().await {
            stdout()
                .write_all(&[line.as_bytes(), b"\n"].concat())
                .await?;
        }
        Ok(())
    }

    pub fn spawn_stdout_task(&self) -> JoinHandle<Result<(), Error>> {
        let stdout = self.clone();
        spawn(async move { stdout.stdout_task().await })
    }
}
