//! Existing `USearch` graph plus a private, rebuildable SQLite vector cache.
//!
//! SQLite transactions persist embeddings before a caller acknowledges canonical
//! work. The graph is an optional checksummed checkpoint: interrupted writes rebuild
//! it from cached vectors without repeating model inference. Queries must supply
//! canonical, already-scoped IDs and exact source revisions; stale vectors cannot
//! use up the result limit. This index never reads mail or changes Seen state.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read as _,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};
use usearch::{Index as Graph, IndexOptions, MetricKind, ScalarKind};

use super::Error;

use mailstronaut_models::{DIMENSIONS, MAX_WINDOWS as WINDOWS};
const MAX_GRAPH_BYTES: u64 = 1024 * 1024 * 1024;
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS state (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1), policy TEXT NOT NULL,
 generation INTEGER NOT NULL, graph_generation INTEGER, checksum TEXT);
CREATE TABLE IF NOT EXISTS vectors (
 id INTEGER PRIMARY KEY CHECK(id>0), source TEXT NOT NULL,
 coordinates BLOB NOT NULL, truncated INTEGER NOT NULL CHECK(truncated IN (0,1)));
";

fn storage(_: impl std::fmt::Debug) -> Error {
    Error::Storage
}

const fn options() -> IndexOptions {
    IndexOptions {
        dimensions: DIMENSIONS,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        connectivity: 32,
        expansion_add: 128,
        expansion_search: 2048,
        multi: true,
    }
}

/// One original's revisioned embedding. Bodies and queries are never stored here.
pub struct Embedding {
    /// Positive canonical message ID.
    pub id: i64,
    /// Hash of the exact source projection used for inference.
    pub source: String,
    /// One through eight normalized 384-dimensional windows, in source order.
    pub vectors: Vec<Vec<f32>>,
    /// More readable source text existed beyond the encoder's bounds.
    pub truncated: bool,
}

/// Cached model vectors and the current graph, owned by one blocking worker.
/// Methods must not run on the UI or asynchronous executor thread.
pub struct Index {
    connection: Connection,
    _lock: fs::File,
    graph: Graph,
    sources: BTreeMap<i64, String>,
    directory: PathBuf,
    healthy: bool,
}

impl Index {
    /// Open a private derived directory for one exact model/projection policy.
    /// A policy change invalidates cached vectors; graph corruption/missing
    /// checkpoints rebuild from transactionally cached vectors without inference.
    /// # Errors
    /// Returns a redacted storage error. Does not delete an unreadable database.
    pub fn open(directory: &Path, policy: &str) -> Result<Self, Error> {
        if policy.is_empty() || policy.len() > 256 {
            return Err(Error::Storage);
        }
        fs::create_dir_all(directory).map_err(storage)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(storage)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(directory.join("writer.lock"))
            .map_err(storage)?;
        lock.try_lock().map_err(|_| Error::Busy)?;
        let path = directory.join("vectors.sqlite");
        // Set private permissions at creation, independently of the process umask.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(storage(error)),
        }
        if !fs::symlink_metadata(&path).map_err(storage)?.is_file() {
            return Err(Error::Storage);
        }
        let mut connection = Connection::open(path).map_err(storage)?;
        connection.execute_batch(SCHEMA).map_err(storage)?;
        let previous: Option<String> = connection
            .query_row("SELECT policy FROM state WHERE singleton=1", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage)?;
        if previous.as_deref() != Some(policy) {
            let tx = connection.transaction().map_err(storage)?;
            tx.execute("DELETE FROM vectors", []).map_err(storage)?;
            tx.execute(
                "INSERT OR REPLACE INTO state VALUES (1,?1,0,NULL,NULL)",
                [policy],
            )
            .map_err(storage)?;
            tx.commit().map_err(storage)?;
        }
        let graph = Graph::new(&options()).map_err(storage)?;
        let mut index = Self {
            connection,
            _lock: lock,
            graph,
            sources: BTreeMap::new(),
            directory: directory.to_path_buf(),
            healthy: true,
        };
        index.restore()?;
        Ok(index)
    }

    fn restore(&mut self) -> Result<(), Error> {
        let capacity: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(SUM(length(coordinates)/?1),0) FROM vectors",
                [i64::try_from(DIMENSIONS * 4).map_err(storage)?],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let capacity = usize::try_from(capacity).map_err(storage)?;
        // One million bounded windows already approaches the 1 GiB graph screen.
        if capacity > 1_000_000 {
            return Err(Error::Storage);
        }
        let checksum: Option<String> = self.connection.query_row("SELECT CASE WHEN graph_generation=generation THEN checksum END FROM state WHERE singleton=1", [], |row| row.get(0)).map_err(storage)?;
        let graph_path = self.directory.join("vectors.usearch");
        let loaded = checksum
            .is_some_and(|expected| digest(&graph_path).is_ok_and(|actual| expected == actual))
            && graph_path
                .to_str()
                .is_some_and(|path| self.graph.load(path).is_ok());
        // Loading validates the library metadata too; an incompatible cache is rebuilt.
        let loaded = loaded
            && self.graph.dimensions() == DIMENSIONS
            && self.graph.multi()
            && self.graph.size() == capacity;
        if !loaded {
            self.graph = Graph::new(&options()).map_err(storage)?;
            self.graph.reserve(capacity.max(1)).map_err(storage)?;
        }
        let mut query = self
            .connection
            .prepare("SELECT id,source,coordinates FROM vectors ORDER BY id")
            .map_err(storage)?;
        let mut rows = query.query([]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            let id: i64 = row.get(0).map_err(storage)?;
            self.sources.insert(id, row.get(1).map_err(storage)?);
            if !loaded {
                let coordinates: Vec<u8> = row.get(2).map_err(storage)?;
                for vector in decode(&coordinates)? {
                    self.graph
                        .add(u64::try_from(id).map_err(storage)?, &vector)
                        .map_err(storage)?;
                }
            }
        }
        Ok(())
    }

    /// Persist one complete replacement, including all its windows. On any error
    /// discard this owner and reopen; partial native mutations are never queried.
    /// # Errors
    /// Rejects invalid vectors/revisions and reports redacted storage failures.
    pub fn put(&mut self, embedding: Embedding) -> Result<(), Error> {
        if !self.healthy {
            return Err(Error::Storage);
        }
        let coordinates = encode(&embedding)?;
        self.healthy = false;
        let key = u64::try_from(embedding.id).map_err(storage)?;
        self.graph.remove(key).map_err(storage)?;
        let capacity = self
            .graph
            .size()
            .checked_add(embedding.vectors.len())
            .ok_or(Error::Storage)?;
        self.graph.reserve(capacity).map_err(storage)?;
        for vector in &embedding.vectors {
            self.graph.add(key, vector).map_err(storage)?;
        }
        let tx = self.connection.transaction().map_err(storage)?;
        tx.execute(
            "INSERT OR REPLACE INTO vectors VALUES (?1,?2,?3,?4)",
            params![
                embedding.id,
                embedding.source,
                coordinates,
                embedding.truncated
            ],
        )
        .map_err(storage)?;
        tx.execute(
            "UPDATE state SET generation=generation+1 WHERE singleton=1",
            [],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        self.sources.insert(embedding.id, embedding.source);
        self.healthy = true;
        Ok(())
    }

    /// Remove every chunk for an original, transactionally from the vector cache.
    /// # Errors
    /// Reports a redacted storage failure; discard this owner after an error.
    pub fn remove(&mut self, id: i64) -> Result<(), Error> {
        if !self.healthy {
            return Err(Error::Storage);
        }
        let key = u64::try_from(id).map_err(storage)?;
        self.healthy = false;
        self.graph.remove(key).map_err(storage)?;
        let tx = self.connection.transaction().map_err(storage)?;
        tx.execute("DELETE FROM vectors WHERE id=?1", [id])
            .map_err(storage)?;
        tx.execute(
            "UPDATE state SET generation=generation+1 WHERE singleton=1",
            [],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)?;
        self.sources.remove(&id);
        self.healthy = true;
        Ok(())
    }

    /// Whether a replayed canonical change already has this exact source revision.
    /// This permits durable queue replay without repeating expensive inference.
    /// # Errors
    /// A failed mutation requires reopening before consulting cached state.
    pub fn contains(&self, id: i64, source: &str) -> Result<bool, Error> {
        if !self.healthy {
            return Err(Error::Storage);
        }
        Ok(self.sources.get(&id).is_some_and(|stored| stored == source))
    }

    /// Search allowed canonical revisions before applying the bounded message cap.
    /// The caller materializes enabled-account/folder/date/sender restrictions from
    /// the canonical store; a different source revision is never a match.
    /// # Errors
    /// Rejects invalid query vectors/limits and reports a redacted index failure.
    pub fn search(
        &self,
        query: &[f32],
        allowed: &BTreeMap<i64, String>,
        limit: usize,
    ) -> Result<Vec<i64>, Error> {
        if !self.healthy || !valid(query) || !(1..=1000).contains(&limit) {
            return Err(Error::Storage);
        }
        if allowed.is_empty() {
            return Ok(Vec::new());
        }
        let hits = self
            .graph
            .filtered_search(query, limit * WINDOWS, |key| {
                i64::try_from(key).is_ok_and(|id| {
                    allowed
                        .get(&id)
                        .is_some_and(|source| self.sources.get(&id) == Some(source))
                })
            })
            .map_err(storage)?;
        let mut seen = BTreeSet::new();
        Ok(hits
            .keys
            .into_iter()
            .filter_map(|key| i64::try_from(key).ok())
            .filter(|id| seen.insert(*id))
            .take(limit)
            .collect())
    }

    /// Save an optional atomic graph checkpoint. A crash at any point leaves the
    /// authoritative cached vectors readable; a mismatched graph is ignored.
    /// Call at idle/checkpoint boundaries, not after every message.
    /// # Errors
    /// Reports a redacted storage failure; never acknowledges canonical work itself.
    pub fn checkpoint(&mut self) -> Result<(), Error> {
        if !self.healthy
            || u64::try_from(self.graph.serialized_length()).map_err(storage)? > MAX_GRAPH_BYTES
        {
            return Err(Error::Storage);
        }
        let temporary = tempfile::NamedTempFile::new_in(&self.directory).map_err(storage)?;
        self.graph
            .save(temporary.path().to_str().ok_or(Error::Storage)?)
            .map_err(storage)?;
        temporary.as_file().sync_all().map_err(storage)?;
        let checksum = digest(temporary.path())?;
        temporary
            .persist(self.directory.join("vectors.usearch"))
            .map_err(storage)?;
        self.connection
            .execute(
                "UPDATE state SET graph_generation=generation,checksum=?1 WHERE singleton=1",
                [checksum],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// Original and explicitly truncated-original counts, without loading text.
    /// # Errors
    /// Reports redacted storage failures.
    pub fn coverage(&self) -> Result<(u64, u64), Error> {
        let (count, truncated): (i64, i64) = self
            .connection
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(truncated),0) FROM vectors",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(storage)?;
        Ok((
            u64::try_from(count).map_err(storage)?,
            u64::try_from(truncated).map_err(storage)?,
        ))
    }
}

fn valid(vector: &[f32]) -> bool {
    vector.len() == DIMENSIONS
        && vector.iter().all(|value| value.is_finite())
        && (vector.iter().map(|value| value * value).sum::<f32>() - 1.).abs() < 0.001
}
fn encode(embedding: &Embedding) -> Result<Vec<u8>, Error> {
    if embedding.id <= 0
        || embedding.source.len() != 64
        || !embedding
            .source
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !(1..=WINDOWS).contains(&embedding.vectors.len())
        || !embedding.vectors.iter().all(|vector| valid(vector))
    {
        return Err(Error::Storage);
    }
    Ok(embedding
        .vectors
        .iter()
        .flatten()
        .flat_map(|value| value.to_le_bytes())
        .collect())
}
fn decode(bytes: &[u8]) -> Result<Vec<Vec<f32>>, Error> {
    if bytes.is_empty()
        || !bytes.len().is_multiple_of(DIMENSIONS * 4)
        || bytes.len() > WINDOWS * DIMENSIONS * 4
    {
        return Err(Error::Storage);
    }
    bytes
        .as_chunks::<{ DIMENSIONS * 4 }>()
        .0
        .iter()
        .map(|window| {
            let vector: Vec<_> = window
                .as_chunks::<4>()
                .0
                .iter()
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();
            if !valid(&vector) {
                return Err(Error::Storage);
            }
            Ok(vector)
        })
        .collect()
}
fn digest(path: &Path) -> Result<String, Error> {
    let mut file = fs::File::open(path).map_err(storage)?;
    if !file.metadata().map_err(storage)?.is_file()
        || file.metadata().map_err(storage)?.len() > MAX_GRAPH_BYTES
    {
        return Err(Error::Storage);
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let length = file.read(&mut buffer).map_err(storage)?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(axis: usize) -> Vec<f32> {
        let mut vector = vec![0.; DIMENSIONS];
        vector[axis] = 1.;
        vector
    }
    fn entry(id: i64, axis: usize) -> Embedding {
        Embedding {
            id,
            source: "a".repeat(64),
            vectors: vec![vector(axis)],
            truncated: false,
        }
    }
    fn allowed(ids: &[i64]) -> BTreeMap<i64, String> {
        ids.iter().map(|id| (*id, "a".repeat(64))).collect()
    }
    #[test]
    fn revisions_and_scope_are_checked_before_the_limit_and_windows_are_deduplicated() {
        let directory = tempfile::tempdir().expect("private cache");
        let mut index = Index::open(directory.path(), "policy-a").expect("index");
        let mut first = entry(1, 0);
        first.vectors.push(vector(1));
        first.truncated = true;
        index.put(first).expect("first");
        index.put(entry(2, 2)).expect("second");
        index.put(entry(3, 0)).expect("foreign");
        assert_eq!(
            index
                .search(&vector(0), &allowed(&[1, 2]), 2)
                .expect("query"),
            [1, 2]
        );
        let mut scope = allowed(&[1, 2]);
        scope.insert(1, "b".repeat(64));
        assert_eq!(
            index.search(&vector(0), &scope, 1).expect("revision fence"),
            [2]
        );
        assert_eq!(
            index
                .search(&vector(0), &BTreeMap::new(), 1)
                .expect("empty"),
            Vec::<i64>::new()
        );
        assert_eq!(index.coverage().expect("counts"), (3, 1));
        assert!(index.contains(1, &"a".repeat(64)).expect("replay"));
        assert!(!index.contains(1, &"b".repeat(64)).expect("changed source"));
        assert!(index.search(&[f32::NAN; DIMENSIONS], &scope, 1).is_err());
    }
    #[test]
    fn a_second_writer_is_refused_and_failed_commits_cannot_answer_searches() {
        let directory = tempfile::tempdir().expect("private cache");
        let mut index = Index::open(directory.path(), "policy-a").expect("index");
        assert!(matches!(
            Index::open(directory.path(), "policy-a"),
            Err(Error::Busy)
        ));
        index.put(entry(1, 0)).expect("first");
        let mut replacement = entry(1, 1);
        replacement.source = "b".repeat(64);
        index.put(replacement).expect("replace all old windows");
        assert_eq!(
            index
                .search(&vector(0), &allowed(&[1]), 1)
                .expect("old revision"),
            Vec::<i64>::new()
        );
        let scope = std::iter::once((1, "b".repeat(64))).collect();
        assert_eq!(
            index.search(&vector(1), &scope, 1).expect("new revision"),
            [1]
        );
        index
            .connection
            .execute("DROP TABLE vectors", [])
            .expect("simulate write failure");
        assert!(index.put(entry(2, 0)).is_err());
        assert!(index.search(&vector(1), &scope, 1).is_err());
        assert!(index.checkpoint().is_err());
        assert!(index.put(entry(3, 0)).is_err());
    }

    #[test]
    fn missing_or_damaged_checkpoint_rebuilds_durable_vectors_and_model_changes_invalidate() {
        let directory = tempfile::tempdir().expect("private cache");
        let mut index = Index::open(directory.path(), "policy-a").expect("index");
        index.put(entry(1, 0)).expect("first");
        // No checkpoint: the cache transaction alone preserves the expensive embedding.
        drop(index);
        let mut index = Index::open(directory.path(), "policy-a").expect("rebuild");
        assert_eq!(
            index.search(&vector(0), &allowed(&[1]), 1).expect("query"),
            [1]
        );
        index.checkpoint().expect("checkpoint");
        drop(index);
        let mut index = Index::open(directory.path(), "policy-a").expect("checkpoint reopen");
        assert_eq!(
            index.search(&vector(0), &allowed(&[1]), 1).expect("query"),
            [1]
        );
        index.put(entry(2, 1)).expect("after checkpoint");
        drop(index);
        let mut index = Index::open(directory.path(), "policy-a").expect("stale checkpoint");
        assert_eq!(
            index
                .search(&vector(1), &allowed(&[1, 2]), 1)
                .expect("query"),
            [2]
        );
        index.checkpoint().expect("updated");
        drop(index);
        fs::write(directory.path().join("vectors.usearch"), b"corrupt graph").expect("damage");
        let mut index = Index::open(directory.path(), "policy-a").expect("checksum recovery");
        assert_eq!(
            index
                .search(&vector(0), &allowed(&[1, 2]), 1)
                .expect("query"),
            [1]
        );
        index.remove(1).expect("remove");
        index.checkpoint().expect("removed checkpoint");
        drop(index);
        let index = Index::open(directory.path(), "policy-a").expect("removed reopen");
        assert_eq!(
            index
                .search(&vector(0), &allowed(&[1, 2]), 2)
                .expect("query"),
            [2]
        );
        drop(index);
        let index = Index::open(directory.path(), "policy-b").expect("new policy");
        assert_eq!(index.coverage().expect("counts"), (0, 0));
        assert_eq!(
            index
                .search(&vector(0), &allowed(&[1, 2]), 2)
                .expect("query"),
            Vec::<i64>::new()
        );
    }
}
