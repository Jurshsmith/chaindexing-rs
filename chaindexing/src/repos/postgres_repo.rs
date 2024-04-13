use std::sync::Arc;

mod migrations;
mod raw_queries;

use crate::chain_reorg::UnsavedReorgedBlock;
use crate::get_contract_addresses_stream_by_chain;
use crate::reset_counts::ResetCount;

use crate::{
    contracts::{ContractAddress, UnsavedContractAddress},
    events::Event,
    nodes::Node,
    Streamable,
};
use diesel_async::RunQueryDsl;

use diesel::{
    delete,
    result::{DatabaseErrorKind, Error as DieselError},
    upsert::excluded,
    ExpressionMethods, OptionalExtension, QueryDsl,
};
use diesel_async::{pooled_connection::AsyncDieselConnectionManager, AsyncPgConnection};
use futures_core::{future::BoxFuture, Stream};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::repo::{Repo, RepoError};

pub type Conn<'a> = bb8::PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;
pub type Pool = bb8::Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;

pub use diesel_async::{
    scoped_futures::ScopedFutureExt as PostgresRepoTransactionExt,
    AsyncConnection as PostgresRepoAsyncConnection,
};

pub use raw_queries::{PostgresRepoRawQueryClient, PostgresRepoRawQueryTxnClient};

impl From<DieselError> for RepoError {
    fn from(value: DieselError) -> Self {
        match value {
            DieselError::DatabaseError(DatabaseErrorKind::ClosedConnection, _info) => {
                RepoError::NotConnected
            }
            any_other_error => RepoError::Unknown(any_other_error.to_string()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PostgresRepo {
    url: String,
}

type PgPooledConn<'a> = bb8::PooledConnection<'a, AsyncDieselConnectionManager<AsyncPgConnection>>;

impl PostgresRepo {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Repo for PostgresRepo {
    type Conn<'a> = PgPooledConn<'a>;
    type Pool = bb8::Pool<AsyncDieselConnectionManager<AsyncPgConnection>>;

    async fn get_pool(&self, max_size: u32) -> Pool {
        let manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(&self.url);

        bb8::Pool::builder().max_size(max_size).build(manager).await.unwrap()
    }

    async fn get_conn<'a>(pool: &'a Pool) -> Conn<'a> {
        pool.get().await.unwrap()
    }

    async fn run_in_transaction<'a, F>(conn: &mut Conn<'a>, repo_ops: F) -> Result<(), RepoError>
    where
        F: for<'b> FnOnce(&'b mut Conn<'a>) -> BoxFuture<'b, Result<(), RepoError>>
            + Send
            + Sync
            + 'a,
    {
        conn.transaction::<(), RepoError, _>(|transaction_conn| {
            async move { (repo_ops)(transaction_conn).await }.scope_boxed()
        })
        .await
    }

    async fn upsert_contract_addresses<'a>(
        conn: &mut Conn<'a>,
        contract_addresses: &[UnsavedContractAddress],
    ) {
        use crate::diesel::schema::chaindexing_contract_addresses::dsl::*;

        diesel::insert_into(chaindexing_contract_addresses)
            .values(contract_addresses)
            .on_conflict((chain_id, address))
            .do_update()
            .set((
                contract_name.eq(excluded(contract_name)),
                start_block_number.eq(excluded(start_block_number)),
            ))
            .execute(conn)
            .await
            .unwrap();
    }

    async fn get_all_contract_addresses<'a>(conn: &mut Conn<'a>) -> Vec<ContractAddress> {
        use crate::diesel::schema::chaindexing_contract_addresses::dsl::*;

        chaindexing_contract_addresses.load(conn).await.unwrap()
    }

    async fn create_events<'a>(conn: &mut Conn<'a>, events: &[Event]) {
        use crate::diesel::schema::chaindexing_events::dsl::*;

        diesel::insert_into(chaindexing_events)
            .values(events)
            .execute(conn)
            .await
            .unwrap();
    }
    async fn get_all_events<'a>(conn: &mut Conn<'a>) -> Vec<Event> {
        use crate::diesel::schema::chaindexing_events::dsl::*;

        chaindexing_events.load(conn).await.unwrap()
    }
    async fn get_events<'a>(
        conn: &mut Self::Conn<'a>,
        address: String,
        from: u64,
        to: u64,
    ) -> Vec<Event> {
        use crate::diesel::schema::chaindexing_events::dsl::*;

        chaindexing_events
            .filter(contract_address.eq(address.to_lowercase()))
            .filter(block_number.between(from as i64, to as i64))
            .load(conn)
            .await
            .unwrap()
    }
    async fn delete_events_by_ids<'a>(conn: &mut Self::Conn<'a>, ids: &[Uuid]) {
        use crate::diesel::schema::chaindexing_events::dsl::*;

        delete(chaindexing_events).filter(id.eq_any(ids)).execute(conn).await.unwrap();
    }

    async fn update_next_block_number_to_ingest_from<'a>(
        conn: &mut Self::Conn<'a>,
        contract_address: &ContractAddress,
        block_number: i64,
    ) {
        use crate::diesel::schema::chaindexing_contract_addresses::dsl::*;

        diesel::update(chaindexing_contract_addresses)
            .filter(id.eq(contract_address.id))
            .set(next_block_number_to_ingest_from.eq(block_number))
            .execute(conn)
            .await
            .unwrap();
    }

    async fn create_reorged_block<'a>(
        conn: &mut Self::Conn<'a>,
        reorged_block: &UnsavedReorgedBlock,
    ) {
        use crate::diesel::schema::chaindexing_reorged_blocks::dsl::*;

        diesel::insert_into(chaindexing_reorged_blocks)
            .values(reorged_block)
            .execute(conn)
            .await
            .unwrap();
    }

    async fn create_reset_count<'a>(conn: &mut Self::Conn<'a>) {
        use crate::diesel::schema::chaindexing_reset_counts::dsl::*;

        diesel::insert_into(chaindexing_reset_counts)
            .default_values()
            .execute(conn)
            .await
            .unwrap();
    }

    async fn get_last_reset_count<'a>(conn: &mut Self::Conn<'a>) -> Option<ResetCount> {
        use crate::diesel::schema::chaindexing_reset_counts::dsl::*;

        chaindexing_reset_counts
            .order_by(id.desc())
            .first(conn)
            .await
            .optional()
            .unwrap()
    }

    async fn create_node<'a>(conn: &mut Self::Conn<'a>) -> Node {
        use crate::diesel::schema::chaindexing_nodes::dsl::*;

        diesel::insert_into(chaindexing_nodes)
            .default_values()
            .get_result(conn)
            .await
            .unwrap()
    }
    async fn get_active_nodes<'a>(
        conn: &mut Self::Conn<'a>,
        node_election_rate_ms: u64,
    ) -> Vec<Node> {
        use crate::diesel::schema::chaindexing_nodes::dsl::*;

        chaindexing_nodes
            .filter(last_active_at.gt(Node::get_min_active_at_in_secs(node_election_rate_ms)))
            .load(conn)
            .await
            .unwrap()
    }
    async fn keep_node_active<'a>(conn: &mut Self::Conn<'a>, node: &Node) {
        use crate::diesel::schema::chaindexing_nodes::dsl::*;

        let now = chrono::offset::Utc::now().timestamp();

        diesel::update(chaindexing_nodes)
            .filter(id.eq(node.id))
            .set(last_active_at.eq(now))
            .execute(conn)
            .await
            .unwrap();
    }
}

impl Streamable for PostgresRepo {
    type StreamConn<'a> = PgPooledConn<'a>;

    fn get_contract_addresses_stream_by_chain<'a>(
        conn: Arc<Mutex<Self::StreamConn<'a>>>,
        chain_id_: i64,
    ) -> Box<dyn Stream<Item = Vec<ContractAddress>> + Send + Unpin + 'a> {
        use crate::diesel::schema::chaindexing_contract_addresses::dsl::*;

        get_contract_addresses_stream_by_chain!(
            id,
            conn,
            Arc<Mutex<PgPooledConn<'a>>>,
            ContractAddress,
            chain_id_,
            i32
        )
    }
}
