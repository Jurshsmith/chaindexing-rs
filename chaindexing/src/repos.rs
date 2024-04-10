#[cfg(feature = "postgres")]
mod postgres_repo;

#[cfg(feature = "postgres")]
pub use postgres_repo::{
    Conn as PostgresRepoConn, Pool as PostgresRepoPool, PostgresRepo, PostgresRepoAsyncConnection,
    PostgresRepoRawQueryClient, PostgresRepoRawQueryTxnClient,
};

mod repo;

pub use repo::{ExecutesWithRawQuery, HasRawQueryClient, Repo, RepoError};

pub(crate) use repo::{
    LoadsDataWithRawQuery, Migratable, RepoMigrations, SQLikeMigrations, Streamable,
};

mod streams;
