//! 1,000 concurrent users against an isolated temp SQLite + HTTP server.
//! Allowlisted metadata only — no raw paths, prompts, or file contents.
use serde_json::{json, Value};
use st_core::verify::{bucket_hash, GENESIS};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const USERS: usize = 1_000;

fn allowlisted_bucket(user_idx: usize, seq: u64) -> Value {
    let file_id = format!("{:016x}", (user_idx as u64).wrapping_mul(0x9e37) ^ seq);
    let mut b = json!({
        "bucket_id": format!("u{user_idx}-s{seq}"),
        "bucket_secs": 60,
        "bucket_start_wall": 1_700_000_000i64 + (seq as i64) * 60,
        "mono_delta_ms": (seq as i64) * 60_000,
        "counts": {
            "human_edits": 1,
            "active_secs": 30,
            "executions": 0
        },
        "files": [{
            "file_id": &file_id[..16],
            "op": "modified",
            "language": "rs"
        }],
        "prev_hash": GENESIS
    });
    let h = bucket_hash(&b);
    b.as_object_mut().unwrap().insert("hash".into(), json!(h));
    b
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn stress_1000_concurrent_users() {
    let dir = std::env::temp_dir().join(format!(
        "st-stress-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = dir.join("stress.db");
    let (addr, handle) = st_server::serve_temp(db.clone()).await.expect("serve_temp");
    let base = format!("http://{addr}");

    // Wait for listener
    for _ in 0..50 {
        if reqwest::get(format!("{base}/health")).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .pool_max_idle_per_host(USERS)
        .build()
        .expect("client");

    // Provision 1k users (bounded concurrency to avoid thundering create_key)
    let mut tokens = Vec::with_capacity(USERS);
    let sem = Arc::new(tokio::sync::Semaphore::new(64));
    let mut joins = Vec::with_capacity(USERS);
    for i in 0..USERS {
        let client = client.clone();
        let base = base.clone();
        let permit = Arc::clone(&sem).acquire_owned().await.unwrap();
        joins.push(tokio::spawn(async move {
            let _permit = permit;
            let handle = format!("u{i:04}");
            let res = client
                .post(format!("{base}/v1/keys"))
                .json(&json!({"handle": handle}))
                .send()
                .await
                .expect("keys");
            assert!(res.status().is_success(), "create_key {}", res.status());
            let body: Value = res.json().await.expect("keys json");
            body["token"].as_str().unwrap().to_string()
        }));
    }
    for j in joins {
        tokens.push(j.await.expect("join key"));
    }
    assert_eq!(tokens.len(), USERS);

    let errors = Arc::new(AtomicU64::new(0));
    let ok = Arc::new(AtomicU64::new(0));
    let t0 = Instant::now();

    // 1k concurrent: heartbeat + ingest + overview
    let mut wave = Vec::with_capacity(USERS);
    for (i, token) in tokens.into_iter().enumerate() {
        let client = client.clone();
        let base = base.clone();
        let errors = Arc::clone(&errors);
        let ok = Arc::clone(&ok);
        wave.push(tokio::spawn(async move {
            // heartbeat
            let h = client.get(format!("{base}/health")).send().await;
            if h.as_ref().map(|r| r.status().is_success()).unwrap_or(false) {
                ok.fetch_add(1, Ordering::Relaxed);
            } else {
                errors.fetch_add(1, Ordering::Relaxed);
            }

            let bucket = allowlisted_bucket(i, 1);
            let batch = client
                .post(format!("{base}/v1/buckets/batch"))
                .header("Authorization", format!("Bearer {token}"))
                .json(&json!({ "buckets": [bucket] }))
                .send()
                .await;
            match batch {
                Ok(r) if r.status().is_success() => {
                    ok.fetch_add(1, Ordering::Relaxed);
                }
                _ => {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }

            let overview = client
                .get(format!("{base}/v1/me/overview"))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await;
            if overview
                .as_ref()
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                ok.fetch_add(1, Ordering::Relaxed);
            } else {
                errors.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for j in wave {
        let _ = j.await;
    }
    let wall = t0.elapsed();

    let metrics: Value = client
        .get(format!("{base}/v1/metrics"))
        .send()
        .await
        .expect("metrics")
        .json()
        .await
        .expect("metrics json");

    let err_n = errors.load(Ordering::Relaxed);
    let ok_n = ok.load(Ordering::Relaxed);
    let rss_kb = read_rss_kb();

    eprintln!("=== StruggleTime 1k-user stress ===");
    eprintln!("users={USERS}");
    eprintln!("wall_s={:.3}", wall.as_secs_f64());
    eprintln!("client_ok={ok_n} client_errors={err_n}");
    eprintln!(
        "server requests={} errors={} req/s={} buckets/s={}",
        metrics["requests"], metrics["errors"], metrics["req_per_sec"], metrics["buckets_per_sec"]
    );
    eprintln!(
        "latency_ms p50={} p95={} p99={} max={}",
        metrics["p50_ms"], metrics["p95_ms"], metrics["p99_ms"], metrics["max_ms"]
    );
    eprintln!(
        "sqlite_ops={} avg_lock_wait_ms={}",
        metrics["sqlite_ops"], metrics["sqlite_avg_lock_wait_ms"]
    );
    if let Some(rss) = rss_kb {
        eprintln!("rss_mb={:.1}", rss as f64 / 1024.0);
    }

    handle.abort();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(err_n, 0, "client-visible errors under 1k concurrency");
    assert_eq!(
        metrics["errors"].as_u64().unwrap_or(1),
        0,
        "server 5xx under 1k concurrency: {metrics}"
    );
    assert!(
        metrics["buckets_accepted"].as_u64().unwrap_or(0) >= USERS as u64,
        "expected >= {USERS} buckets accepted: {metrics}"
    );
}

#[tokio::test]
async fn invalid_bucket_is_rejected_without_db_write() {
    let dir = std::env::temp_dir().join(format!(
        "st-allowlist-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = dir.join("allowlist.db");
    let (addr, handle) = st_server::serve_temp(db.clone()).await.expect("serve_temp");
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let key = client
        .post(format!("{base}/v1/keys"))
        .json(&json!({"handle": "allowlist-test"}))
        .send()
        .await
        .expect("create key");
    assert!(key.status().is_success());
    let key: Value = key.json().await.expect("key json");
    let token = key["token"].as_str().expect("token");

    let valid = allowlisted_bucket(0, 1);
    let response = client
        .post(format!("{base}/v1/buckets/batch"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "buckets": [valid] }))
        .send()
        .await
        .expect("valid ingest");
    assert!(response.status().is_success());

    let mut invalid = allowlisted_bucket(0, 2);
    invalid["counts"]["agent_suggestions"] = json!(1);
    let hash = bucket_hash(&invalid);
    invalid
        .as_object_mut()
        .unwrap()
        .insert("hash".into(), json!(hash));
    let response = client
        .post(format!("{base}/v1/buckets/batch"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "buckets": [invalid] }))
        .send()
        .await
        .expect("invalid ingest");
    assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);

    let conn = rusqlite::Connection::open(&db).expect("open verification db");
    let stored: i64 = conn
        .query_row("SELECT COUNT(*) FROM buckets", [], |row| row.get(0))
        .expect("count stored buckets");
    assert_eq!(stored, 1, "invalid bucket must not be persisted");
    drop(conn);

    handle.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

fn read_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}
