use crate::config::{Config, normalize};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

pub fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().as_bytes().to_vec()
    }
}
pub fn native_path(bytes: Vec<u8>) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(bytes).into()
    }
    #[cfg(not(unix))]
    {
        String::from_utf8_lossy(&bytes).to_string().into()
    }
}

pub struct Catalog {
    pub db: Connection,
    pub path: PathBuf,
    _lock: File,
}

#[derive(Clone, Debug)]
pub struct Source {
    pub run: i64,
    pub sequence: i64,
    pub relative: PathBuf,
    pub size: u64,
    pub birth: Option<i64>,
    pub modified: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Content {
    pub digest: String,
    pub size: u64,
    pub source: PathBuf,
    pub intended: PathBuf,
    pub stage: Option<PathBuf>,
    pub temporary: PathBuf,
    pub complete: bool,
    pub date: Option<String>,
    pub provenance: String,
}

impl Catalog {
    pub fn all_contents(&self) -> Result<Vec<Content>> {
        let mut statement=self.db.prepare("SELECT digest,size,representative,intended,stage,temporary,state,date,provenance FROM contents ORDER BY rowid")?;
        Ok(statement
            .query_map([], content_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn open(path: &Path, create: bool) -> Result<Self> {
        let path = normalize(path)?;
        ensure!(
            create || path.is_file(),
            "catalog does not exist: {}",
            path.display()
        );
        if create {
            crate::publication::create_parents(
                path.parent().context("catalog has no parent")?,
                &mut |_| Ok(()),
            )?;
        }
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(Path::new(&lock_path))?;
        FileExt::try_lock_exclusive(&lock).context("catalog is locked by another command")?;
        let db = Connection::open(&path)?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(
            (0..=2).contains(&version),
            "unsupported catalog schema version {version}"
        );
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;",
        )?;
        if version == 0 {
            db.execute_batch("BEGIN IMMEDIATE;
                CREATE TABLE metadata(key TEXT PRIMARY KEY, value BLOB NOT NULL);
                CREATE TABLE runs(id INTEGER PRIMARY KEY, source BLOB NOT NULL, destination BLOB NOT NULL,
                    staging BLOB NOT NULL, budget INTEGER NOT NULL CHECK(budget>0), phase TEXT NOT NULL DEFAULT 'inventory',
                    total_files INTEGER, total_bytes INTEGER, next_position INTEGER NOT NULL DEFAULT 0,
                    skipped_symlinks INTEGER NOT NULL DEFAULT 0, skipped_special INTEGER NOT NULL DEFAULT 0, error TEXT);
                CREATE TABLE source_files(run INTEGER NOT NULL REFERENCES runs(id), sequence INTEGER NOT NULL,
                    relative BLOB NOT NULL, extension BLOB, size INTEGER NOT NULL, birth INTEGER, modified INTEGER,
                    digest TEXT REFERENCES contents(digest), error TEXT, PRIMARY KEY(run,sequence));
                CREATE TABLE contents(digest TEXT PRIMARY KEY, size INTEGER NOT NULL, representative BLOB NOT NULL,
                    intended BLOB NOT NULL UNIQUE, stage BLOB, temporary BLOB NOT NULL, state TEXT NOT NULL DEFAULT 'pending',
                    date TEXT, provenance TEXT NOT NULL, write_sequence INTEGER UNIQUE, error TEXT);
                CREATE INDEX source_dependencies ON source_files(digest);
                CREATE INDEX copy_order ON contents(state,write_sequence);
                CREATE TABLE verification_passes(id INTEGER PRIMARY KEY, run INTEGER NOT NULL REFERENCES runs(id),
                    state TEXT NOT NULL DEFAULT 'running', bytes INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE verification_results(pass INTEGER NOT NULL REFERENCES verification_passes(id),
                    digest TEXT NOT NULL REFERENCES contents(digest), success INTEGER NOT NULL, error TEXT,
                    PRIMARY KEY(pass,digest));
                PRAGMA user_version=1; COMMIT;")?;
        }
        if version < 2 {
            db.execute_batch(
                "BEGIN IMMEDIATE;
                CREATE TABLE run_reports(run INTEGER PRIMARY KEY REFERENCES runs(id),
                    generated_at TEXT NOT NULL, report TEXT NOT NULL);
                PRAGMA user_version=2; COMMIT;",
            )?;
        }
        db.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS content_temporary ON contents(temporary);",
        )?;
        if create {
            // Persist the catalog's directory entry before accepting import work.
            crate::publication::sync_directory(path.parent().context("catalog has no parent")?)?;
        }
        Ok(Self {
            db,
            path,
            _lock: lock,
        })
    }

    pub fn associate(&self, destination: &Path) -> Result<()> {
        let normalized = normalize(destination)?;
        let previous: Option<Vec<u8>> = self
            .db
            .query_row(
                "SELECT value FROM metadata WHERE key='destination'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            ensure!(
                previous == path_bytes(&normalized),
                "catalog destination cannot change"
            );
        } else {
            self.db.execute(
                "INSERT INTO metadata VALUES('destination',?1)",
                [path_bytes(&normalized)],
            )?;
        }
        Ok(())
    }

    pub fn new_run(&self, config: &Config) -> Result<i64> {
        self.associate(&config.destination)?;
        self.db.execute(
            "INSERT INTO runs(source,destination,staging,budget) VALUES(?1,?2,?3,?4)",
            params![
                path_bytes(&config.source),
                path_bytes(&config.destination),
                path_bytes(&config.staging),
                config.budget as i64
            ],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    pub fn config(&self, run: i64) -> Result<Config> {
        self.db
            .query_row(
                "SELECT source,destination,staging,budget FROM runs WHERE id=?1",
                [run],
                |r| {
                    Ok(Config {
                        source: native_path(r.get(0)?),
                        destination: native_path(r.get(1)?),
                        staging: native_path(r.get(2)?),
                        budget: r.get::<_, i64>(3)? as u64,
                        catalog: self.path.clone(),
                    })
                },
            )
            .with_context(|| format!("unknown run {run}"))
    }
    pub fn phase(&self, run: i64) -> Result<String> {
        Ok(self
            .db
            .query_row("SELECT phase FROM runs WHERE id=?1", [run], |r| r.get(0))?)
    }
    pub fn totals(&self, run: i64) -> Result<(u64, u64)> {
        Ok(self.db.query_row(
            "SELECT total_files,total_bytes FROM runs WHERE id=?1",
            [run],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )?)
    }
    pub fn source_rows(&self, run: i64, only_unclaimed: bool) -> Result<Vec<Source>> {
        let mut statement = self.db.prepare(
            "SELECT run,sequence,relative,size,birth,modified FROM source_files
            WHERE run=?1 AND (?2=0 OR digest IS NULL) ORDER BY sequence",
        )?;
        Ok(statement
            .query_map(params![run, only_unclaimed], |r| {
                Ok(Source {
                    run: r.get(0)?,
                    sequence: r.get(1)?,
                    relative: native_path(r.get(2)?),
                    size: r.get::<_, i64>(3)? as u64,
                    birth: r.get(4)?,
                    modified: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn content(&self, digest: &str) -> Result<Option<Content>> {
        Ok(self.db.query_row("SELECT digest,size,representative,intended,stage,temporary,state,date,provenance FROM contents WHERE digest=?1",
            [digest], content_row).optional()?)
    }
    pub fn path_reserved(&self, path: &Path) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM contents WHERE intended=?1 OR temporary=?1)",
            [path_bytes(path)],
            |r| r.get(0),
        )?)
    }
    pub fn claim(&mut self, source: &Source, content: &Content) -> Result<bool> {
        let tx = self.db.transaction()?;
        let inserted = tx.execute("INSERT OR IGNORE INTO contents(digest,size,representative,intended,stage,temporary,date,provenance)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![content.digest,content.size as i64,path_bytes(&content.source),
                path_bytes(&content.intended),content.stage.as_deref().map(path_bytes),path_bytes(&content.temporary),content.date,content.provenance])?;
        let known_size: i64 = tx.query_row(
            "SELECT size FROM contents WHERE digest=?1",
            [&content.digest],
            |r| r.get(0),
        )?;
        ensure!(known_size as u64 == source.size, "digest size conflict");
        tx.execute(
            "UPDATE source_files SET digest=?1,error=NULL WHERE run=?2 AND sequence=?3",
            params![content.digest, source.run, source.sequence],
        )?;
        tx.execute(
            "UPDATE runs SET next_position=MAX(next_position,?1) WHERE id=?2",
            params![source.sequence + 1, source.run],
        )?;
        tx.commit()?;
        Ok(inserted != 0)
    }
    pub fn complete(&mut self, digest: &str) -> Result<()> {
        let tx = self.db.transaction()?;
        tx.execute("UPDATE contents SET state='complete',error=NULL,
            write_sequence=(SELECT COALESCE(MAX(write_sequence),0)+1 FROM contents) WHERE digest=?1 AND state!='complete'", [digest])?;
        tx.commit()?;
        Ok(())
    }
    pub fn done(&self, run: i64) -> Result<u64> {
        Ok(self.db.query_row("SELECT COUNT(*) FROM source_files s JOIN contents c ON s.digest=c.digest WHERE s.run=?1 AND c.state='complete'", [run], |r|r.get::<_,i64>(0))? as u64)
    }
    pub fn contents_for_run(&self, run: i64) -> Result<Vec<Content>> {
        let mut stmt = self.db.prepare("SELECT digest,size,representative,intended,stage,temporary,state,date,provenance FROM contents
            WHERE digest IN (SELECT digest FROM source_files WHERE run=?1) ORDER BY write_sequence,intended")?;
        Ok(stmt
            .query_map([run], content_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn error(&self, run: i64, message: &str) -> Result<()> {
        self.db.execute(
            "UPDATE runs SET error=?1 WHERE id=?2",
            params![message, run],
        )?;
        Ok(())
    }
}

fn content_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Content> {
    Ok(Content {
        digest: r.get(0)?,
        size: r.get::<_, i64>(1)? as u64,
        source: native_path(r.get(2)?),
        intended: native_path(r.get(3)?),
        stage: r.get::<_, Option<Vec<u8>>>(4)?.map(native_path),
        temporary: native_path(r.get(5)?),
        complete: r.get::<_, String>(6)? == "complete",
        date: r.get(7)?,
        provenance: r.get(8)?,
    })
}
