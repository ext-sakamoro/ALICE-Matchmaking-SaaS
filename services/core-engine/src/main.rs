use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use uuid::Uuid;

#[derive(Debug, Default)]
struct Stats {
    total_ops: u64,
    find_ops: u64,
    rate_ops: u64,
    queue_ops: u64,
}

#[derive(Debug, Clone)]
struct QueueEntry {
    player_id: String,
    rating: f64,
    mode: String,
}

#[derive(Debug, Default)]
struct AppData {
    stats: Stats,
    queue: VecDeque<QueueEntry>,
    total_matches: u64,
    avg_wait_secs: f64,
    avg_quality: f64,
}

type AppState = Arc<Mutex<AppData>>;

// --- /api/v1/match/find ---
#[derive(Debug, Deserialize)]
struct FindRequest {
    players: Vec<PlayerInfo>,
    mode: String,
    #[serde(default = "default_range")]
    rating_range: f64,
}

fn default_range() -> f64 {
    200.0
}

#[derive(Debug, Deserialize)]
struct PlayerInfo {
    id: String,
    rating: f64,
}

#[derive(Debug, Serialize)]
struct FindResponse {
    request_id: String,
    match_id: String,
    team_a: Vec<String>,
    team_b: Vec<String>,
    avg_rating: f64,
    quality_score: f64,
    estimated_wait_secs: f64,
}

async fn find_match(
    State(state): State<AppState>,
    Json(req): Json<FindRequest>,
) -> Result<Json<FindResponse>, StatusCode> {
    if req.players.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let avg_rating = req.players.iter().map(|p| p.rating).sum::<f64>() / req.players.len() as f64;
    let max_dev = req
        .players
        .iter()
        .map(|p| (p.rating - avg_rating).abs())
        .fold(0.0_f64, f64::max);
    let quality_score = ((1.0 - max_dev / req.rating_range.max(1.0)) * 100.0)
        .clamp(0.0, 100.0)
        .round();

    let half = req.players.len() / 2;
    let team_a: Vec<String> = req.players[..half].iter().map(|p| p.id.clone()).collect();
    let team_b: Vec<String> = req.players[half..].iter().map(|p| p.id.clone()).collect();

    {
        let mut d = state.lock().unwrap();
        d.stats.total_ops += 1;
        d.stats.find_ops += 1;
        d.total_matches += 1;
        d.avg_quality = (d.avg_quality * (d.total_matches - 1) as f64 + quality_score)
            / d.total_matches as f64;
    }

    Ok(Json(FindResponse {
        request_id: Uuid::new_v4().to_string(),
        match_id: Uuid::new_v4().to_string(),
        team_a,
        team_b,
        avg_rating: (avg_rating * 10.0).round() / 10.0,
        quality_score,
        estimated_wait_secs: 3.5,
    }))
}

// --- /api/v1/match/rate ---
#[derive(Debug, Deserialize)]
struct RateRequest {
    player_id: String,
    current_rating: f64,
    opponent_rating: f64,
    score: f64, // 1.0 win, 0.5 draw, 0.0 loss
    k_factor: Option<f64>,
}

#[derive(Debug, Serialize)]
struct RateResponse {
    request_id: String,
    player_id: String,
    old_rating: f64,
    new_rating: f64,
    delta: f64,
}

async fn update_rating(
    State(state): State<AppState>,
    Json(req): Json<RateRequest>,
) -> Result<Json<RateResponse>, StatusCode> {
    {
        let mut d = state.lock().unwrap();
        d.stats.total_ops += 1;
        d.stats.rate_ops += 1;
    }
    let k = req.k_factor.unwrap_or(32.0);
    let expected = 1.0 / (1.0 + 10.0_f64.powf((req.opponent_rating - req.current_rating) / 400.0));
    let delta = k * (req.score - expected);
    let new_rating = (req.current_rating + delta).max(100.0);

    Ok(Json(RateResponse {
        request_id: Uuid::new_v4().to_string(),
        player_id: req.player_id,
        old_rating: req.current_rating,
        new_rating: (new_rating * 10.0).round() / 10.0,
        delta: (delta * 10.0).round() / 10.0,
    }))
}

// --- /api/v1/match/queue ---
#[derive(Debug, Deserialize)]
struct QueueRequest {
    player_id: String,
    rating: f64,
    mode: String,
}

#[derive(Debug, Serialize)]
struct QueueResponse {
    request_id: String,
    player_id: String,
    position: usize,
    queue_size: usize,
    estimated_wait_secs: f64,
}

async fn join_queue(
    State(state): State<AppState>,
    Json(req): Json<QueueRequest>,
) -> Result<Json<QueueResponse>, StatusCode> {
    let (position, queue_size) = {
        let mut d = state.lock().unwrap();
        d.stats.total_ops += 1;
        d.stats.queue_ops += 1;
        d.queue.push_back(QueueEntry {
            player_id: req.player_id.clone(),
            rating: req.rating,
            mode: req.mode.clone(),
        });
        let qs = d.queue.len();
        (qs, qs)
    };

    Ok(Json(QueueResponse {
        request_id: Uuid::new_v4().to_string(),
        player_id: req.player_id,
        position,
        queue_size,
        estimated_wait_secs: position as f64 * 2.5,
    }))
}

// --- /api/v1/match/quality ---
#[derive(Debug, Serialize)]
struct QualityResponse {
    service: &'static str,
    avg_match_quality: f64,
    total_matches: u64,
    current_queue_size: usize,
    avg_wait_secs: f64,
}

async fn match_quality(State(state): State<AppState>) -> Json<QualityResponse> {
    let d = state.lock().unwrap();
    Json(QualityResponse {
        service: "alice-matchmaking-core",
        avg_match_quality: (d.avg_quality * 10.0).round() / 10.0,
        total_matches: d.total_matches,
        current_queue_size: d.queue.len(),
        avg_wait_secs: d.avg_wait_secs,
    })
}

// --- /api/v1/match/stats ---
#[derive(Debug, Serialize)]
struct StatsResponse {
    service: &'static str,
    version: &'static str,
    total_ops: u64,
    find_ops: u64,
    rate_ops: u64,
    queue_ops: u64,
    total_matches: u64,
}

async fn get_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    let d = state.lock().unwrap();
    Json(StatsResponse {
        service: "alice-matchmaking-core",
        version: env!("CARGO_PKG_VERSION"),
        total_ops: d.stats.total_ops,
        find_ops: d.stats.find_ops,
        rate_ops: d.stats.rate_ops,
        queue_ops: d.stats.queue_ops,
        total_matches: d.total_matches,
    })
}

// --- /health ---
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
    total_ops: u64,
}

async fn health(
    State(state): State<AppState>,
    axum::extract::Extension(start): axum::extract::Extension<Arc<Instant>>,
) -> Json<HealthResponse> {
    let d = state.lock().unwrap();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: start.elapsed().as_secs(),
        total_ops: d.stats.total_ops,
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let state: AppState = Arc::new(Mutex::new(AppData::default()));
    let start = Arc::new(Instant::now());

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/match/find", post(find_match))
        .route("/api/v1/match/rate", post(update_rating))
        .route("/api/v1/match/queue", post(join_queue))
        .route("/api/v1/match/quality", get(match_quality))
        .route("/api/v1/match/stats", get(get_stats))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .layer(axum::extract::Extension(start))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8124);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("alice-matchmaking-core listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
