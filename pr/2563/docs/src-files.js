var srcIndex = new Map(JSON.parse('[\
["bulk",["",[],["bulk.rs"]]],\
["iroh",["",[["client",[],["authors.rs","blobs.rs","docs.rs","gossip.rs","node.rs","quic.rs","tags.rs"]],["node",[["rpc",[],["docs.rs"]]],["builder.rs","docs.rs","nodes_storage.rs","protocol.rs","rpc.rs","rpc_status.rs"]],["rpc_protocol",[],["authors.rs","blobs.rs","docs.rs","gossip.rs","node.rs","tags.rs"]],["util",[],["fs.rs","io.rs","path.rs","progress.rs"]]],["client.rs","lib.rs","metrics.rs","node.rs","rpc_protocol.rs","util.rs"]]],\
["iroh_base",["",[["key",[],["encryption.rs"]],["ticket",[],["blob.rs","node.rs"]]],["base32.rs","hash.rs","key.rs","lib.rs","node_addr.rs","rpc.rs","ticket.rs"]]],\
["iroh_blobs",["",[["downloader",[],["get.rs","invariants.rs","progress.rs"]],["format",[],["collection.rs"]],["get",[],["db.rs","error.rs","progress.rs","request.rs"]],["protocol",[],["range_spec.rs"]],["store",[["fs",[],["import_flat_store.rs","migrate_redb_v1_v2.rs","tables.rs","test_support.rs","util.rs","validate.rs"]]],["bao_file.rs","fs.rs","mem.rs","mutable_mem_storage.rs","readonly_mem.rs","traits.rs"]],["util",[],["io.rs","local_pool.rs","mem_or_file.rs","progress.rs","sparse_mem_file.rs"]]],["downloader.rs","export.rs","format.rs","get.rs","hashseq.rs","lib.rs","metrics.rs","protocol.rs","provider.rs","store.rs","util.rs"]]],\
["iroh_dns_server",["",[["dns",[],["node_authority.rs"]],["http",[["doh",[],["extract.rs","response.rs"]]],["doh.rs","error.rs","pkarr.rs","rate_limiting.rs","tls.rs"]],["store",[],["signed_packets.rs"]]],["config.rs","dns.rs","http.rs","lib.rs","metrics.rs","server.rs","state.rs","store.rs","util.rs"]]],\
["iroh_docs",["",[["engine",[],["gossip.rs","live.rs","state.rs"]],["net",[],["codec.rs"]],["store",[["fs",[],["bounds.rs","migrate_v1_v2.rs","migrations.rs","query.rs","ranges.rs","tables.rs"]]],["fs.rs","pubkeys.rs","util.rs"]]],["actor.rs","engine.rs","heads.rs","keys.rs","lib.rs","metrics.rs","net.rs","ranger.rs","store.rs","sync.rs","ticket.rs"]]],\
["iroh_gossip",["",[["net",[],["util.rs"]],["proto",[],["hyparview.rs","plumtree.rs","state.rs","topic.rs","util.rs"]]],["dispatcher.rs","lib.rs","metrics.rs","net.rs","proto.rs"]]],\
["iroh_metrics",["",[],["core.rs","lib.rs","metrics.rs","service.rs"]]],\
["iroh_net",["",[["discovery",[],["dns.rs","local_swarm_discovery.rs","pkarr.rs"]],["dns",[],["node_info.rs"]],["endpoint",[],["rtt_actor.rs"]],["magicsock",[["node_map",[],["best_addr.rs","node_state.rs"]]],["metrics.rs","node_map.rs","relay_actor.rs","timer.rs","udp_conn.rs"]],["net",[["interfaces",[],["linux.rs"]],["netmon",[],["actor.rs","linux.rs"]]],["interfaces.rs","ip.rs","ip_family.rs","netmon.rs","udp.rs"]],["netcheck",[["reportgen",[],["hairpin.rs","probes.rs"]]],["metrics.rs","reportgen.rs"]],["portmapper",[["nat_pmp",[["protocol",[],["request.rs","response.rs"]]],["protocol.rs"]],["pcp",[["protocol",[],["opcode_data.rs","request.rs","response.rs"]]],["protocol.rs"]]],["current_mapping.rs","mapping.rs","metrics.rs","nat_pmp.rs","pcp.rs","upnp.rs"]],["relay",[["http",[],["client.rs","server.rs","streams.rs"]]],["client.rs","client_conn.rs","clients.rs","codec.rs","http.rs","iroh_relay.rs","map.rs","metrics.rs","server.rs","types.rs"]],["tls",[],["certificate.rs","verifier.rs"]],["util",[],["chain.rs"]]],["defaults.rs","dialer.rs","disco.rs","discovery.rs","dns.rs","endpoint.rs","lib.rs","magicsock.rs","metrics.rs","net.rs","netcheck.rs","ping.rs","portmapper.rs","relay.rs","stun.rs","test_utils.rs","ticket.rs","tls.rs","util.rs"]]],\
["iroh_net_bench",["",[],["iroh.rs","lib.rs","quinn.rs","s2n.rs","stats.rs"]]],\
["iroh_relay",["",[],["iroh-relay.rs"]]],\
["iroh_test",["",[],["hexdump.rs","lib.rs","logging.rs"]]]\
]'));
createSrcSidebar();
