use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, PgPool};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use std::{str::FromStr, time::Duration};

#[derive(Deserialize, Serialize)]
struct PostTransaction {
    #[serde(rename = "valor")]
    value: i32,
    #[serde(rename = "tipo")]
    kind: TransactionKind,
    #[serde(rename = "descricao")]
    description: String,
}

#[derive(sqlx::Type, Deserialize, Serialize, Debug)]
struct Transaction {
    #[serde(rename = "valor")]
    value: i32,
    #[serde(rename = "tipo")]
    kind: TransactionKind,
    #[serde(rename = "descricao")]
    description: String,
    #[serde(rename = "realizada_em")]
    inserted_at: OffsetDateTime,
}

#[derive(sqlx::Type, Debug, Serialize, Deserialize)]
#[sqlx(type_name = "transaction_kind", rename_all = "lowercase")]
enum TransactionKind {
    #[serde(rename = "c")]
    Credit,
    #[serde(rename = "d")]
    Debit,
}

impl FromStr for TransactionKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "c" => Ok(TransactionKind::Credit),
            "d" => Ok(TransactionKind::Debit),
            _ => Err(format!("Invalid transaction kind: {}", s)),
        }
    }
}

impl ToString for TransactionKind {
    fn to_string(&self) -> String {
        match self {
            TransactionKind::Credit => "c".to_string(),
            TransactionKind::Debit => "d".to_string(),
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_tokio_postgres=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db_connection_str = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://rinha:rinha@localhost/rinha".to_string());

    // set up connection pool
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&db_connection_str)
        .await
        .expect("can't connect to database");

    // run migrations
    // sqlx::migrate!().run(&pool).await.unwrap();

    // build our application with some routes
    let app = Router::new()
        .route("/", get(hello_world))
        .route("/clientes/:id/extrato", get(statement))
        .route("/clientes/:id/transacoes", post(insert_transaction))
        .with_state(pool);

    // run it with hyper
    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

async fn hello_world() -> String {
    "Hello, World!".to_string()
}

// async fn statement(
//     State(pool): State<PgPool>,
//     Path(wallet_id): Path<i32>,
// ) -> Result<(), (StatusCode, String)> {
//     let foo = sqlx::query!(
//         r#"
//         SELECT w.balance, w.credit_limit,
//         ARRAY_AGG((t.value, t.kind, t.description, t.inserted_at)) as "transactions: Vec<Transaction>"
//         FROM wallets w
//         INNER JOIN transactions t ON w.id = t.wallet_id
//         WHERE w.id = $1
//         GROUP BY w.balance, w.credit_limit;
//         "#,
//         wallet_id
//     )
//     .fetch_one(&pool)
//     .await
//     .map_err(internal_error);

//     println!("{:?}", foo);

//     Ok(())
// }

async fn statement(
    State(pool): State<PgPool>,
    Path(wallet_id): Path<i32>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let wallet = sqlx::query!(
        r#"
        SELECT balance, credit_limit
        FROM wallets
        WHERE id = $1
        "#,
        wallet_id
    )
    .fetch_one(&pool)
    .await
    .map_err(not_found)?;

    let transactions = sqlx::query!(
        r#"
        SELECT value, kind as "kind: TransactionKind", description, inserted_at
        FROM transactions
        WHERE wallet_id = $1
        ORDER BY inserted_at DESC
        LIMIT 10;
        "#,
        wallet_id
    )
    .fetch_all(&pool)
    .await
    .map_err(unprocessable_entity)?;

    let mut v = Vec::new();

    for row in transactions {
        v.push(json!({
            "valor": row.value,
            "tipo": row.kind,
            "descricao": row.description,
            "realizada_em": row.inserted_at.expect("error parsing date").format(&Rfc3339).unwrap(),
        }));
    }

    Ok(Json(json!({
        "saldo": {
            "total": wallet.balance,
            "data_extrato": OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
            "limite": wallet.credit_limit
        },
        "ultimas_transacoes": v

    })))
}

async fn insert_transaction(
    Path(wallet_id): Path<i32>,
    State(pool): State<PgPool>,
    Json(post_transaction): Json<PostTransaction>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut transaction = pool.begin().await.map_err(internal_error)?;

    let updated_value = match &post_transaction.kind {
        TransactionKind::Credit => post_transaction.value,
        TransactionKind::Debit => -post_transaction.value,
    };

    let _ = sqlx::query!(
        r#"
        INSERT INTO transactions (wallet_id, value, kind, description) VALUES ($1, $2, $3, $4);
        "#,
        wallet_id,
        post_transaction.value,
        post_transaction.kind as _,
        post_transaction.description
    )
    .execute(&mut *transaction)
    .await
    .map_err(unprocessable_entity)?;

    let res = sqlx::query!(
        r#"
        UPDATE wallets SET balance = balance + $1 WHERE id = $2 RETURNING balance, credit_limit;
        "#,
        updated_value,
        wallet_id
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(unprocessable_entity)?;

    transaction.commit().await.map_err(unprocessable_entity)?;

    Ok(Json(json!({
        "saldo": res.balance,
        "limite": res.credit_limit
    })))
}

fn internal_error<E>(err: E) -> (StatusCode, String)
where
    E: std::error::Error,
{
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn unprocessable_entity<E>(err: E) -> (StatusCode, String)
where
    E: std::error::Error,
{
    (StatusCode::UNPROCESSABLE_ENTITY, err.to_string())
}

fn not_found<E>(err: E) -> (StatusCode, String)
where
    E: std::error::Error,
{
    (StatusCode::NOT_FOUND, err.to_string())
}