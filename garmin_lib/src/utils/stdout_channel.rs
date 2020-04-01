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
    receiver: Arc<Mutex<UnboundedReceiver<String>>>,
    sender: Arc<UnboundedSender<String>>,
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
        self.sender.send(item).map_err(Into::into)
    }

    pub async fn recv(&self) -> Option<String> {
        self.receiver.lock().await.recv().await
    }

    pub async fn close(&self) {
        self.receiver.lock().await.close()
    }

    async fn stdout_task(&self) -> Result<(), Error> {
        while let Some(line) = self.recv().await {
            stdout().write_all(line.as_bytes()).await?;
            stdout().write_all(b"\n").await?;
        }
        Ok(())
    }

    pub fn spawn_stdout_task(&self) -> JoinHandle<Result<(), Error>> {
        let stdout = self.clone();
        spawn(async move { stdout.stdout_task().await })
    }
}
