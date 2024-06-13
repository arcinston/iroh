use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use futures_lite::future::Boxed as BoxedFuture;
use iroh::{
    blobs::store::Store,
    net::{
        endpoint::{get_remote_node_id, Connecting},
        NodeId,
    },
    node::{Node, Protocol},
};
use tracing_subscriber::{prelude::*, EnvFilter};

#[derive(Debug, Parser)]
pub struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Parser)]
pub enum Command {
    Accept,
    Connect { node: NodeId },
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging();
    let args = Cli::parse();
    // create a new node
    let node = iroh::node::Node::memory()
        .accept(EXAMPLE_ALPN, |node| {
            Box::pin(async move { Ok(ExampleProto::build(node)) })
        })
        .spawn()
        .await?;

    // print the ticket if this is the accepting side
    match args.command {
        Command::Accept => {
            let node_id = node.node_id();
            println!("node id: {node_id}");
            // wait until ctrl-c
            tokio::signal::ctrl_c().await?;
        }
        Command::Connect { node: node_id } => {
            let proto = ExampleProto::get_from_node(&node, EXAMPLE_ALPN).expect("it is registered");
            proto.connect(node_id).await?;
        }
    }

    node.shutdown().await?;

    Ok(())
}

const EXAMPLE_ALPN: &[u8] = b"example-proto/0";

#[derive(Debug)]
struct ExampleProto<S> {
    node: Node<S>,
}

impl<S: Store> Protocol for ExampleProto<S> {
    fn accept(self: Arc<Self>, connecting: Connecting) -> BoxedFuture<Result<()>> {
        Box::pin(async move {
            let connection = connecting.await?;
            let peer = get_remote_node_id(&connection)?;
            println!("accepted connection from {peer}");
            let mut send_stream = connection.open_uni().await?;
            // Let's create a new blob for each incoming connection.
            // This functions as an example of using existing iroh functionality within a protocol
            // (you likely don't want to create a new blob for each connection for real)
            let content = format!("this blob is created for my beloved peer {peer} ♥");
            let hash = self
                .node
                .blobs()
                .add_bytes(content.as_bytes().to_vec())
                .await?;
            // Send the hash over our custom protocol.
            send_stream.write_all(hash.hash.as_bytes()).await?;
            send_stream.finish().await?;
            println!("closing connection from {peer}");
            Ok(())
        })
    }
}

impl<S: Store> ExampleProto<S> {
    pub fn build(node: Node<S>) -> Arc<dyn Protocol> {
        Arc::new(Self { node })
    }

    pub fn get_from_node(node: &Node<S>, alpn: &'static [u8]) -> Option<Arc<Self>> {
        node.get_protocol::<ExampleProto<S>>(alpn)
    }

    pub async fn connect(&self, remote_node_id: NodeId) -> Result<()> {
        println!("our node id: {}", self.node.node_id());
        println!("connecting to {remote_node_id}");
        let conn = self
            .node
            .endpoint()
            .connect_by_node_id(&remote_node_id, EXAMPLE_ALPN)
            .await?;
        let mut recv_stream = conn.accept_uni().await?;
        let hash_bytes = recv_stream.read_to_end(32).await?;
        let hash = iroh::blobs::Hash::from_bytes(hash_bytes.try_into().unwrap());
        println!("received hash: {hash}");
        self.node
            .blobs()
            .download(hash, remote_node_id.into())
            .await?
            .await?;
        println!("blob downloaded");
        let content = self.node.blobs().read_to_bytes(hash).await?;
        let message = String::from_utf8(content.to_vec())?;
        println!("blob content: {message}");
        Ok(())
    }
}

/// Set the RUST_LOG env var to one of {debug,info,warn} to see logging.
fn setup_logging() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(EnvFilter::from_default_env())
        .try_init()
        .ok();
}
