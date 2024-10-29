use std::{
    collections::BTreeMap,
    net::SocketAddr,
    ops::Range,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use bao_tree::{blake3, ChunkNum, ChunkRanges};
use bytes::Bytes;
use futures_lite::FutureExt;
use iroh::node::{Builder, DocsStorage};
use iroh_base::node_addr::NodeAddrOptions;
use iroh_blobs::{
    format::collection::Collection,
    get::{
        fsm::{self, ConnectedNext, DecodeError},
        Stats,
    },
    protocol::{GetRequest, RangeSpecSeq},
    store::{MapMut, Store},
    BlobFormat, Hash,
};
use iroh_net::{defaults::staging::default_relay_map, key::SecretKey, NodeAddr, NodeId};
use rand::RngCore;

/// Create a new endpoint and dial a peer, returning the connection.
async fn dial(secret_key: SecretKey, peer: NodeAddr) -> anyhow::Result<quinn::Connection> {
    let endpoint = iroh_net::Endpoint::builder()
        .secret_key(secret_key)
        .bind()
        .await?;
    endpoint
        .connect(peer, iroh::blobs::protocol::ALPN)
        .await
        .context("failed to connect to provider")
}

fn test_node<D: Store>(db: D) -> Builder<D> {
    iroh::node::Builder::with_db_and_store(db, DocsStorage::Memory, iroh::node::StorageConfig::Mem)
        .bind_random_port()
}

#[tokio::test]
async fn basics() -> Result<()> {
    let _guard = iroh_test::logging::setup();
    transfer_data(vec![("hello_world", "hello world!".as_bytes().to_vec())]).await
}

#[tokio::test]
async fn multi_file() -> Result<()> {
    let _guard = iroh_test::logging::setup();

    let file_opts = vec![
        ("1", 10),
        ("2", 1024),
        ("3", 1024 * 1024),
        // overkill, but it works! Just annoying to wait for
        // ("4", 1024 * 1024 * 90),
    ];
    transfer_random_data(file_opts).await
}

#[tokio::test]
async fn many_files() -> Result<()> {
    let _guard = iroh_test::logging::setup();
    let num_files = [10, 100];
    for num in num_files {
        println!("NUM_FILES: {num}");
        let file_opts = (0..num)
            .map(|i| {
                // use a long file name to test large collections
                let name = i.to_string().repeat(50);
                (name, 10)
            })
            .collect();
        transfer_random_data(file_opts).await?;
    }
    Ok(())
}

#[tokio::test]
async fn sizes() -> Result<()> {
    let _guard = iroh_test::logging::setup();

    let sizes = [
        0,
        10,
        100,
        1024,
        1024 * 100,
        1024 * 500,
        1024 * 1024,
        1024 * 1024 + 10,
        1024 * 1024 * 9,
    ];

    for size in sizes {
        let now = Instant::now();
        transfer_random_data(vec![("hello_world", size)]).await?;
        println!("  took {}ms", now.elapsed().as_millis());
    }

    Ok(())
}

#[tokio::test]
async fn empty_files() -> Result<()> {
    // try to transfer as many files as possible without hitting a limit
    // booo 400 is too small :(
    let num_files = 400;
    let mut file_opts = Vec::new();
    for i in 0..num_files {
        file_opts.push((i.to_string(), 0));
    }
    transfer_random_data(file_opts).await
}

/// Create new get options with the given node id and addresses, using a
/// randomly generated secret key.
fn get_options(
    node_id: NodeId,
    addrs: impl IntoIterator<Item = SocketAddr>,
) -> (SecretKey, NodeAddr) {
    let relay_map = default_relay_map();
    let peer = iroh_net::NodeAddr::from_parts(
        node_id,
        relay_map.nodes().next().map(|n| n.url.clone()),
        addrs,
    );
    (SecretKey::generate(), peer)
}

#[tokio::test(flavor = "multi_thread")]
async fn multiple_clients() -> Result<()> {
    let content = b"hello world!";

    let mut db = iroh_blobs::store::readonly_mem::Store::default();
    let expect_hash = db.insert(content.as_slice());
    let expect_name = "hello_world";
    let collection = Collection::from_iter([(expect_name, expect_hash)]);
    let hash = db.insert_many(collection.to_blobs()).unwrap();
    let node = test_node(db).spawn().await?;
    let mut tasks = Vec::new();
    for _i in 0..3 {
        let file_hash: Hash = expect_hash;
        let name = expect_name;
        let addrs = node.local_address();
        let peer_id = node.node_id();
        let content = content.to_vec();

        tasks.push(node.local_pool_handle().spawn(move || {
            async move {
                let (secret_key, peer) = get_options(peer_id, addrs);
                let expected_data = &content;
                let expected_name = name;
                let request = GetRequest::all(hash);
                let (collection, children, _stats) =
                    run_collection_get_request(secret_key, peer, request).await?;
                assert_eq!(expected_name, &collection[0].0);
                assert_eq!(&file_hash, &collection[0].1);
                assert_eq!(expected_data, &children[&0]);

                anyhow::Ok(())
            }
            .boxed_local()
        }));
    }

    futures_buffered::try_join_all(tasks).await?;
    Ok(())
}

// Run the test creating random data for each blob, using the size specified by the file
// options
async fn transfer_random_data<S>(file_opts: Vec<(S, usize)>) -> Result<()>
where
    S: Into<String> + std::fmt::Debug + std::cmp::PartialEq + Clone,
{
    let file_opts = file_opts
        .into_iter()
        .map(|(name, size)| {
            let mut content = vec![0u8; size];
            rand::thread_rng().fill_bytes(&mut content);
            (name, content)
        })
        .collect();
    transfer_data(file_opts).await
}

// Run the test for a vec of filenames and blob data
async fn transfer_data<S>(file_opts: Vec<(S, Vec<u8>)>) -> Result<()>
where
    S: Into<String> + std::fmt::Debug + std::cmp::PartialEq + Clone,
{
    let mut expects = Vec::new();
    let num_blobs = file_opts.len();

    let (mut mdb, _lookup) = iroh_blobs::store::readonly_mem::Store::new(file_opts.clone());
    let mut blobs = Vec::new();

    for opt in file_opts.into_iter() {
        let (name, data) = opt;
        let name: String = name.into();
        println!("Sending {}: {}b", name, data.len());

        // get expected hash of file
        let hash = blake3::hash(&data);
        let hash = Hash::from(hash);
        let blob = (name.clone(), hash);
        blobs.push(blob);

        // keep track of expected values
        expects.push((name, hash));
    }
    let collection_orig = Collection::from_iter(blobs);
    let collection_hash = mdb.insert_many(collection_orig.to_blobs()).unwrap();

    let node = test_node(mdb.clone()).spawn().await?;

    let addrs = node.local_endpoint_addresses().await?;
    let (secret_key, peer) = get_options(node.node_id(), addrs);
    let request = GetRequest::all(collection_hash);
    let (collection, children, _stats) =
        run_collection_get_request(secret_key, peer, request).await?;
    assert_eq!(num_blobs, collection.len());
    for (i, (expected_name, expected_hash)) in expects.iter().enumerate() {
        let (name, hash) = &collection[i];
        let got = &children[&(i as u64)];
        let expected = mdb.get_content(expected_hash).unwrap();
        assert_eq!(expected_name, name);
        assert_eq!(expected_hash, hash);
        assert_eq!(expected, got);
    }

    node.shutdown().await?;

    Ok(())
}

#[tokio::test]
async fn test_server_close() {
    let _guard = iroh_test::logging::setup();

    // Prepare a Provider transferring a file.
    let mut db = iroh_blobs::store::readonly_mem::Store::default();
    let child_hash = db.insert(b"hello there");
    let collection = Collection::from_iter([("hello", child_hash)]);
    let hash = db.insert_many(collection.to_blobs()).unwrap();
    let node = test_node(db).spawn().await.unwrap();
    let node_addr = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();

    let (secret_key, peer) = get_options(peer_id, node_addr);
    let request = GetRequest::all(hash);
    let (_collection, _children, _stats) = run_collection_get_request(secret_key, peer, request)
        .await
        .unwrap();
}

/// create an in memory test database containing the given entries and an iroh collection of all entries
///
/// returns the database and the root hash of the collection
fn create_test_db(
    entries: impl IntoIterator<Item = (impl Into<String>, impl AsRef<[u8]>)>,
) -> (iroh_blobs::store::readonly_mem::Store, Hash) {
    let (mut db, hashes) = iroh_blobs::store::readonly_mem::Store::new(entries);
    let collection = Collection::from_iter(hashes);
    let hash = db.insert_many(collection.to_blobs()).unwrap();
    (db, hash)
}

#[tokio::test]
#[ignore = "flaky"]
async fn test_ipv6() {
    let _guard = iroh_test::logging::setup();

    let (db, hash) = create_test_db([("test", b"hello")]);
    let node = match test_node(db).spawn().await {
        Ok(provider) => provider,
        Err(_) => {
            // We assume the problem here is IPv6 on this host.  If the problem is
            // not IPv6 then other tests will also fail.
            return;
        }
    };
    let addrs = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();
    tokio::time::timeout(Duration::from_secs(10), async move {
        let (secret_key, peer) = get_options(peer_id, addrs);
        let request = GetRequest::all(hash);
        run_collection_get_request(secret_key, peer, request).await
    })
    .await
    .expect("timeout")
    .expect("get failed");
}

/// Simulate a node that has nothing
#[tokio::test]
async fn test_not_found() {
    let _guard = iroh_test::logging::setup();

    let db = iroh_blobs::store::readonly_mem::Store::default();
    let hash = blake3::hash(b"hello").into();
    let node = match test_node(db).spawn().await {
        Ok(provider) => provider,
        Err(_) => {
            // We assume the problem here is IPv6 on this host.  If the problem is
            // not IPv6 then other tests will also fail.
            return;
        }
    };
    let addrs = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();
    tokio::time::timeout(Duration::from_secs(10), async move {
        let (secret_key, peer) = get_options(peer_id, addrs);
        let request = GetRequest::single(hash);
        let res = run_collection_get_request(secret_key, peer, request).await;
        if let Err(cause) = res {
            if let Some(e) = cause.downcast_ref::<DecodeError>() {
                if let DecodeError::NotFound = e {
                    Ok(())
                } else {
                    anyhow::bail!("expected DecodeError::NotFound, got {:?}", e);
                }
            } else {
                anyhow::bail!("expected DecodeError, got {:?}", cause);
            }
        } else {
            anyhow::bail!("expected error when getting non-existent blob");
        }
    })
    .await
    .expect("timeout")
    .expect("get failed");
}

/// Simulate a node that has just begun downloading a blob, but does not yet have any data
#[tokio::test]
async fn test_chunk_not_found_1() {
    let _guard = iroh_test::logging::setup();

    let db = iroh_blobs::store::mem::Store::new();
    let data = (0..1024 * 64).map(|i| i as u8).collect::<Vec<_>>();
    let hash = blake3::hash(&data).into();
    let _entry = db.get_or_create(hash, data.len() as u64).await.unwrap();
    let node = match test_node(db).spawn().await {
        Ok(provider) => provider,
        Err(_) => {
            // We assume the problem here is IPv6 on this host.  If the problem is
            // not IPv6 then other tests will also fail.
            return;
        }
    };
    let addrs = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();
    tokio::time::timeout(Duration::from_secs(10), async move {
        let (secret_key, peer) = get_options(peer_id, addrs);
        let request = GetRequest::single(hash);
        let res = run_collection_get_request(secret_key, peer, request).await;
        if let Err(cause) = res {
            if let Some(e) = cause.downcast_ref::<DecodeError>() {
                if let DecodeError::NotFound = e {
                    Ok(())
                } else {
                    anyhow::bail!("expected DecodeError::ParentNotFound, got {:?}", e);
                }
            } else {
                anyhow::bail!("expected DecodeError, got {:?}", cause);
            }
        } else {
            anyhow::bail!("expected error when getting non-existent blob");
        }
    })
    .await
    .expect("timeout")
    .expect("get failed");
}

#[tokio::test]
async fn test_run_ticket() {
    let _guard = iroh_test::logging::setup();

    let (db, hash) = create_test_db([("test", b"hello")]);
    let node = test_node(db).spawn().await.unwrap();
    let _drop_guard = node.cancel_token().drop_guard();

    let ticket = node
        .blobs()
        .share(
            hash,
            BlobFormat::HashSeq,
            NodeAddrOptions::RelayAndAddresses,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async move {
        let request = GetRequest::all(hash);
        run_collection_get_request(SecretKey::generate(), ticket.node_addr().clone(), request).await
    })
    .await
    .expect("timeout")
    .expect("get ticket failed");
}

/// Utility to validate that the children of a collection are correct
fn validate_children(collection: Collection, children: BTreeMap<u64, Bytes>) -> anyhow::Result<()> {
    let blobs = collection.into_iter().collect::<Vec<_>>();
    anyhow::ensure!(blobs.len() == children.len());
    for (child, (_name, hash)) in blobs.into_iter().enumerate() {
        let child = child as u64;
        let data = children.get(&child).unwrap();
        anyhow::ensure!(hash == blake3::hash(data).into());
    }
    Ok(())
}

async fn run_collection_get_request(
    secret_key: SecretKey,
    peer: NodeAddr,
    request: GetRequest,
) -> anyhow::Result<(Collection, BTreeMap<u64, Bytes>, Stats)> {
    let connection = dial(secret_key, peer).await?;
    let initial = fsm::start(connection, request);
    let connected = initial.next().await?;
    let ConnectedNext::StartRoot(fsm_at_start_root) = connected.next().await? else {
        anyhow::bail!("request did not include collection");
    };
    Collection::read_fsm_all(fsm_at_start_root).await
}

#[tokio::test]
async fn test_run_fsm() {
    let _guard = iroh_test::logging::setup();

    let (db, hash) = create_test_db([("a", b"hello"), ("b", b"world")]);
    let node = test_node(db).spawn().await.unwrap();
    let addrs = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();
    tokio::time::timeout(Duration::from_secs(10), async move {
        let (secret_key, peer) = get_options(peer_id, addrs);
        let request = GetRequest::all(hash);
        let (collection, children, _) =
            run_collection_get_request(secret_key, peer, request).await?;
        validate_children(collection, children)?;
        anyhow::Ok(())
    })
    .await
    .expect("timeout")
    .expect("get failed");
}

/// compute the range of the last chunk of a blob of the given size
fn last_chunk_range(size: usize) -> Range<usize> {
    const CHUNK_LEN: usize = 1024;
    const MASK: usize = CHUNK_LEN - 1;
    if (size & MASK) == 0 {
        size - CHUNK_LEN..size
    } else {
        (size & !MASK)..size
    }
}

fn last_chunk(data: &[u8]) -> &[u8] {
    let range = last_chunk_range(data.len());
    &data[range]
}

fn make_test_data(n: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        data.push((i / 1024) as u8);
    }
    data
}

/// Ask for the last chunk of a blob, even if we don't know the size yet.
///
/// The verified last chunk also verifies the size.
#[tokio::test]
async fn test_size_request_blob() {
    let _guard = iroh_test::logging::setup();

    let expected = make_test_data(1024 * 64 + 1234);
    let last_chunk = last_chunk(&expected);
    let (db, hashes) = iroh_blobs::store::readonly_mem::Store::new([("test", &expected)]);
    let hash = Hash::from(*hashes.values().next().unwrap());
    let node = test_node(db).spawn().await.unwrap();
    let addrs = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();
    tokio::time::timeout(Duration::from_secs(10), async move {
        let request = GetRequest::last_chunk(hash);
        let (secret_key, peer) = get_options(peer_id, addrs);
        let connection = dial(secret_key, peer).await?;
        let response = fsm::start(connection, request);
        let connected = response.next().await?;
        let ConnectedNext::StartRoot(start) = connected.next().await? else {
            panic!()
        };
        let header = start.next();
        let (_, actual) = header.concatenate_into_vec().await?;
        assert_eq!(actual, last_chunk);
        anyhow::Ok(())
    })
    .await
    .expect("timeout")
    .expect("get failed");
}

#[tokio::test]
async fn test_collection_stat() {
    let _guard = iroh_test::logging::setup();

    let child1 = make_test_data(123456);
    let child2 = make_test_data(345678);
    let (db, hash) = create_test_db([("a", &child1), ("b", &child2)]);
    let node = test_node(db.clone()).spawn().await.unwrap();
    let addrs = node.local_endpoint_addresses().await.unwrap();
    let peer_id = node.node_id();
    tokio::time::timeout(Duration::from_secs(10), async move {
        // first 1024 bytes
        let header = ChunkRanges::from(..ChunkNum(1));
        // last chunk, whatever it is, to verify the size
        let end = ChunkRanges::from(ChunkNum(u64::MAX)..);
        // combine them
        let ranges = &header | &end;
        let request = GetRequest::new(
            hash,
            RangeSpecSeq::from_ranges_infinite([ChunkRanges::all(), ranges]),
        );
        let (secret_key, peer) = get_options(peer_id, addrs);
        let (_collection, items, _stats) =
            run_collection_get_request(secret_key, peer, request).await?;
        // we should get the first <=1024 bytes and the last chunk of each child
        // so now we know the size and can guess the type by inspecting the header
        assert_eq!(items.len(), 2);
        assert_eq!(&items[&0][..1024], &child1[..1024]);
        assert!(items[&0].ends_with(last_chunk(&child1)));
        assert_eq!(&items[&1][..1024], &child2[..1024]);
        assert!(items[&1].ends_with(last_chunk(&child2)));
        anyhow::Ok(())
    })
    .await
    .expect("timeout")
    .expect("get failed");
}
