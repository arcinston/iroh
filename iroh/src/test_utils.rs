//! Internal utilities to support testing.
use std::net::Ipv4Addr;

use anyhow::Result;
pub use dns_and_pkarr_servers::DnsPkarrServer;
pub use dns_server::create_dns_resolver;
use iroh_base::{RelayMap, RelayNode, RelayUrl};
use iroh_relay::server::{
    CertConfig, QuicConfig, RelayConfig, Server, ServerConfig, StunConfig, TlsConfig,
};
use tokio::sync::oneshot;

use crate::defaults::DEFAULT_STUN_PORT;

/// A drop guard to clean up test infrastructure.
///
/// After dropping the test infrastructure will asynchronously shutdown and release its
/// resources.
// Nightly sees the sender as dead code currently, but we only rely on Drop of the
// sender.
#[derive(Debug)]
#[allow(dead_code)]
pub struct CleanupDropGuard(pub(crate) oneshot::Sender<()>);

/// Runs a relay server with STUN and QUIC enabled suitable for tests.
///
/// The returned `Url` is the url of the relay server in the returned [`RelayMap`].
/// When dropped, the returned [`Server`] does will stop running.
pub async fn run_relay_server() -> Result<(RelayMap, RelayUrl, Server)> {
    run_relay_server_with(
        Some(StunConfig {
            bind_addr: (Ipv4Addr::LOCALHOST, 0).into(),
        }),
        true,
    )
    .await
}

/// Runs a relay server with STUN enabled suitable for tests.
///
/// The returned `Url` is the url of the relay server in the returned [`RelayMap`].
/// When dropped, the returned [`Server`] does will stop running.
pub async fn run_relay_server_with_stun() -> Result<(RelayMap, RelayUrl, Server)> {
    run_relay_server_with(
        Some(StunConfig {
            bind_addr: (Ipv4Addr::LOCALHOST, 0).into(),
        }),
        false,
    )
    .await
}

/// Runs a relay server.
///
/// `stun` can be set to `None` to disable stun, or set to `Some` `StunConfig`,
/// to enable stun on a specific socket.
///
/// If `quic` is set to `true`, it will make the appropriate [`QuicConfig`] from the generated tls certificates and run the quic server at a random free port.
///
///
/// The return value is similar to [`run_relay_server`].
pub async fn run_relay_server_with(
    stun: Option<StunConfig>,
    quic: bool,
) -> Result<(RelayMap, RelayUrl, Server)> {
    let (certs, server_config) = iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let tls = TlsConfig {
        cert: CertConfig::<(), ()>::Manual { certs },
        https_bind_addr: (Ipv4Addr::LOCALHOST, 0).into(),
        quic_bind_addr: (Ipv4Addr::LOCALHOST, 0).into(),
        server_config,
    };
    let quic = if quic {
        Some(QuicConfig {
            server_config: tls.server_config.clone(),
            bind_addr: tls.quic_bind_addr,
        })
    } else {
        None
    };
    let config = ServerConfig {
        relay: Some(RelayConfig {
            http_bind_addr: (Ipv4Addr::LOCALHOST, 0).into(),
            tls: Some(tls),
            limits: Default::default(),
        }),
        quic,
        stun,
        #[cfg(feature = "metrics")]
        metrics_addr: None,
    };
    let server = Server::spawn(config).await.unwrap();
    let url: RelayUrl = format!("https://{}", server.https_addr().expect("configured"))
        .parse()
        .unwrap();
    let quic = server
        .quic_addr()
        .map(|addr| iroh_base::RelayQuicConfig { port: addr.port() });
    let m = RelayMap::from_nodes([RelayNode {
        url: url.clone(),
        stun_only: false,
        stun_port: server.stun_addr().map_or(DEFAULT_STUN_PORT, |s| s.port()),
        quic,
    }])
    .unwrap();
    Ok((m, url, server))
}

pub(crate) mod dns_and_pkarr_servers {
    use std::{net::SocketAddr, time::Duration};

    use anyhow::Result;
    use iroh_base::{NodeId, SecretKey};
    use url::Url;

    use super::{create_dns_resolver, CleanupDropGuard};
    use crate::{
        discovery::{dns::DnsDiscovery, pkarr::PkarrPublisher, ConcurrentDiscovery},
        dns::DnsResolver,
        test_utils::{
            dns_server::run_dns_server, pkarr_dns_state::State, pkarr_relay::run_pkarr_relay,
        },
    };

    /// Handle and drop guard for test DNS and Pkarr servers.
    ///
    /// Once the struct is dropped the servers will shut down.
    #[derive(Debug)]
    pub struct DnsPkarrServer {
        /// The node origin domain.
        pub node_origin: String,
        /// The shared state of the DNS and Pkarr servers.
        state: State,
        /// The socket address of the DNS server.
        pub nameserver: SocketAddr,
        /// The HTTP URL of the Pkarr server.
        pub pkarr_url: Url,
        _dns_drop_guard: CleanupDropGuard,
        _pkarr_drop_guard: CleanupDropGuard,
    }

    impl DnsPkarrServer {
        /// Run DNS and Pkarr servers on localhost.
        pub async fn run() -> anyhow::Result<Self> {
            Self::run_with_origin("dns.iroh.test".to_string()).await
        }

        /// Run DNS and Pkarr servers on localhost with the specified `node_origin` domain.
        pub async fn run_with_origin(node_origin: String) -> anyhow::Result<Self> {
            let state = State::new(node_origin.clone());
            let (nameserver, dns_drop_guard) = run_dns_server(state.clone()).await?;
            let (pkarr_url, pkarr_drop_guard) = run_pkarr_relay(state.clone()).await?;
            Ok(Self {
                node_origin,
                nameserver,
                pkarr_url,
                state,
                _dns_drop_guard: dns_drop_guard,
                _pkarr_drop_guard: pkarr_drop_guard,
            })
        }

        /// Create a [`ConcurrentDiscovery`] with [`DnsDiscovery`] and [`PkarrPublisher`]
        /// configured to use the test servers.
        pub fn discovery(&self, secret_key: SecretKey) -> Box<ConcurrentDiscovery> {
            Box::new(ConcurrentDiscovery::from_services(vec![
                // Enable DNS discovery by default
                Box::new(DnsDiscovery::new(self.node_origin.clone())),
                // Enable pkarr publishing by default
                Box::new(PkarrPublisher::new(secret_key, self.pkarr_url.clone())),
            ]))
        }

        /// Create a [`DnsResolver`] configured to use the test DNS server.
        pub fn dns_resolver(&self) -> DnsResolver {
            create_dns_resolver(self.nameserver).expect("failed to create DNS resolver")
        }

        /// Wait until a Pkarr announce for a node is published to the server.
        ///
        /// If `timeout` elapses an error is returned.
        pub async fn on_node(&self, node_id: &NodeId, timeout: Duration) -> Result<()> {
            self.state.on_node(node_id, timeout).await
        }
    }
}

pub(crate) mod dns_server {
    use std::{
        future::Future,
        net::{Ipv4Addr, SocketAddr},
    };

    use anyhow::{ensure, Result};
    use futures_lite::future::Boxed as BoxFuture;
    use hickory_resolver::{
        config::NameServerConfig,
        proto::{
            op::{header::MessageType, Message},
            serialize::binary::BinDecodable,
        },
        TokioResolver,
    };
    use tokio::{net::UdpSocket, sync::oneshot};
    use tracing::{debug, error, warn};

    use super::CleanupDropGuard;

    /// Trait used by [`run_dns_server`] for answering DNS queries.
    pub trait QueryHandler: Send + Sync + 'static {
        fn resolve(
            &self,
            query: &Message,
            reply: &mut Message,
        ) -> impl Future<Output = Result<()>> + Send;
    }

    pub type QueryHandlerFunction =
        Box<dyn Fn(&Message, &mut Message) -> BoxFuture<Result<()>> + Send + Sync + 'static>;

    impl QueryHandler for QueryHandlerFunction {
        fn resolve(
            &self,
            query: &Message,
            reply: &mut Message,
        ) -> impl Future<Output = Result<()>> + Send {
            (self)(query, reply)
        }
    }

    /// Run a DNS server.
    ///
    /// Must pass a [`QueryHandler`] that answers queries.
    pub async fn run_dns_server(
        resolver: impl QueryHandler,
    ) -> Result<(SocketAddr, CleanupDropGuard)> {
        let bind_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let socket = UdpSocket::bind(bind_addr).await?;
        let bound_addr = socket.local_addr()?;
        let s = TestDnsServer { socket, resolver };
        let (tx, mut rx) = oneshot::channel();
        tokio::task::spawn(async move {
            tokio::select! {
                _ = &mut rx => {
                    debug!("shutting down dns server");
                }
                res = s.run() => {
                    if let Err(e) = res {
                        error!("error running dns server {e:?}");
                    }
                }
            }
        });
        Ok((bound_addr, CleanupDropGuard(tx)))
    }

    /// Create a DNS resolver with a single nameserver.
    pub fn create_dns_resolver(nameserver: SocketAddr) -> Result<TokioResolver> {
        let mut config = hickory_resolver::config::ResolverConfig::new();
        let nameserver_config =
            NameServerConfig::new(nameserver, hickory_resolver::proto::xfer::Protocol::Udp);
        config.add_name_server(nameserver_config);
        let resolver = hickory_resolver::Resolver::tokio(config, Default::default());
        Ok(resolver)
    }

    struct TestDnsServer<R> {
        resolver: R,
        socket: UdpSocket,
    }

    impl<R: QueryHandler> TestDnsServer<R> {
        async fn run(self) -> Result<()> {
            let mut buf = [0; 1450];
            loop {
                let res = self.socket.recv_from(&mut buf).await;
                let (len, from) = res?;
                if let Err(err) = self.handle_datagram(from, &buf[..len]).await {
                    warn!(?err, %from, "failed to handle incoming datagram");
                }
            }
        }

        async fn handle_datagram(&self, from: SocketAddr, buf: &[u8]) -> Result<()> {
            let packet = Message::from_bytes(buf)?;
            debug!(queries = ?packet.queries(), %from, "received query");
            let mut reply = packet.clone();
            reply.set_message_type(MessageType::Response);
            self.resolver.resolve(&packet, &mut reply).await?;
            debug!(?reply, %from, "send reply");
            let buf = reply.to_vec()?;
            let len = self.socket.send_to(&buf, from).await?;
            ensure!(len == buf.len(), "failed to send complete packet");
            Ok(())
        }
    }
}

pub(crate) mod pkarr_relay {
    use std::{
        future::IntoFuture,
        net::{Ipv4Addr, SocketAddr},
    };

    use anyhow::Result;
    use axum::{
        extract::{Path, State},
        response::IntoResponse,
        routing::put,
        Router,
    };
    use bytes::Bytes;
    use tokio::sync::oneshot;
    use tracing::{debug, error, warn};
    use url::Url;

    use super::CleanupDropGuard;
    use crate::test_utils::pkarr_dns_state::State as AppState;

    pub async fn run_pkarr_relay(state: AppState) -> Result<(Url, CleanupDropGuard)> {
        let bind_addr = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let app = Router::new()
            .route("/pkarr/:key", put(pkarr_put))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        let bound_addr = listener.local_addr()?;
        let url: Url = format!("http://{bound_addr}/pkarr")
            .parse()
            .expect("valid url");

        let (tx, mut rx) = oneshot::channel();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app);
            tokio::select! {
                _ = &mut rx => {
                    debug!("shutting down pkarr server");
                }
                res = serve.into_future() => {
                    if let Err(e) = res {
                        error!("pkarr server error: {e:?}");
                    }
                }
            }
        });
        Ok((url, CleanupDropGuard(tx)))
    }

    async fn pkarr_put(
        State(state): State<AppState>,
        Path(key): Path<String>,
        body: Bytes,
    ) -> Result<impl IntoResponse, AppError> {
        let key = pkarr::PublicKey::try_from(key.as_str())?;
        let signed_packet = pkarr::SignedPacket::from_relay_payload(&key, &body)?;
        let _updated = state.upsert(signed_packet)?;
        Ok(http::StatusCode::NO_CONTENT)
    }

    #[derive(Debug)]
    struct AppError(anyhow::Error);
    impl<T: Into<anyhow::Error>> From<T> for AppError {
        fn from(value: T) -> Self {
            Self(value.into())
        }
    }
    impl IntoResponse for AppError {
        fn into_response(self) -> axum::response::Response {
            warn!(err = ?self, "request failed");
            (http::StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
        }
    }
}

pub(crate) mod pkarr_dns_state {
    use std::{
        collections::{hash_map, HashMap},
        future::Future,
        ops::Deref,
        sync::Arc,
        time::Duration,
    };

    use anyhow::{bail, Result};
    use iroh_base::NodeId;
    use parking_lot::{Mutex, MutexGuard};
    use pkarr::SignedPacket;

    use crate::{
        dns::node_info::{node_id_from_hickory_name, NodeInfo},
        test_utils::dns_server::QueryHandler,
    };

    #[derive(Debug, Clone)]
    pub struct State {
        packets: Arc<Mutex<HashMap<NodeId, SignedPacket>>>,
        origin: String,
        notify: Arc<tokio::sync::Notify>,
    }

    impl State {
        pub fn new(origin: String) -> Self {
            Self {
                packets: Default::default(),
                origin,
                notify: Arc::new(tokio::sync::Notify::new()),
            }
        }

        pub fn on_update(&self) -> tokio::sync::futures::Notified<'_> {
            self.notify.notified()
        }

        pub async fn on_node(&self, node: &NodeId, timeout: Duration) -> Result<()> {
            let timeout = tokio::time::sleep(timeout);
            tokio::pin!(timeout);
            while self.get(node).is_none() {
                tokio::select! {
                    _ = &mut timeout => bail!("timeout"),
                    _ = self.on_update() => {}
                }
            }
            Ok(())
        }

        pub fn upsert(&self, signed_packet: SignedPacket) -> anyhow::Result<bool> {
            let node_id = NodeId::from_bytes(&signed_packet.public_key().to_bytes())?;
            let mut map = self.packets.lock();
            let updated = match map.entry(node_id) {
                hash_map::Entry::Vacant(e) => {
                    e.insert(signed_packet);
                    true
                }
                hash_map::Entry::Occupied(mut e) => {
                    if signed_packet.more_recent_than(e.get()) {
                        e.insert(signed_packet);
                        true
                    } else {
                        false
                    }
                }
            };
            if updated {
                self.notify.notify_waiters();
            }
            Ok(updated)
        }

        /// Returns a mutex guard, do not hold over await points
        pub fn get(&self, node_id: &NodeId) -> Option<impl Deref<Target = SignedPacket> + '_> {
            let map = self.packets.lock();
            if map.contains_key(node_id) {
                let guard = MutexGuard::map(map, |state| state.get_mut(node_id).unwrap());
                Some(guard)
            } else {
                None
            }
        }

        pub fn resolve_dns(
            &self,
            query: &hickory_resolver::proto::op::Message,
            reply: &mut hickory_resolver::proto::op::Message,
            ttl: u32,
        ) -> Result<()> {
            for query in query.queries() {
                let Some(node_id) = node_id_from_hickory_name(query.name()) else {
                    continue;
                };
                let packet = self.get(&node_id);
                let Some(packet) = packet.as_ref() else {
                    continue;
                };
                let node_info = NodeInfo::from_pkarr_signed_packet(packet)?;
                for record in node_info.to_hickory_records(&self.origin, ttl)? {
                    reply.add_answer(record);
                }
            }
            Ok(())
        }
    }

    impl QueryHandler for State {
        fn resolve(
            &self,
            query: &hickory_resolver::proto::op::Message,
            reply: &mut hickory_resolver::proto::op::Message,
        ) -> impl Future<Output = Result<()>> + Send {
            const TTL: u32 = 30;
            let res = self.resolve_dns(query, reply, TTL);
            std::future::ready(res)
        }
    }
}
