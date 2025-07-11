use anyhow::{Result, anyhow};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::SystemTime;

/// Metadata tracked by the session store for each session
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetadata {
    created_at: SystemTime,
    last_used: SystemTime,
}

/// Internal wrapper for session data with metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SessionEntry<T> {
    data: T,
    metadata: SessionMetadata,
}

impl<T> SessionEntry<T> {
    fn update_last_used(&mut self) {
        self.metadata.last_used = SystemTime::now();
    }
}

impl Default for SessionMetadata {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            created_at: now,
            last_used: now,
        }
    }
}

/// Generic session store that handles persistence and cross-process synchronization
///
/// This store automatically watches for file changes from other processes and reloads
/// when needed. From the user's perspective, it behaves like an in-memory HashMap
/// with automatic persistence and cross-process sharing.
///
/// **Note:** This type is intentionally NOT `Clone` to prevent data divergence issues.
/// Cloning would create separate in-memory caches that could become inconsistent,
/// leading to lost updates. Instead, use shared ownership (&mut references) or
/// Arc<Mutex<_>> if you need to share the store across multiple contexts.
#[derive(Debug)]
pub struct SessionStore<T> {
    sessions: HashMap<String, SessionEntry<T>>,
    storage_path: Option<PathBuf>,
    needs_reload: Arc<AtomicBool>,
    ignore_next_events: Arc<AtomicUsize>, // Counter for ignoring our own writes
    _watcher: Option<RecommendedWatcher>, // Keeps the file watcher thread alive
}

impl<T> SessionStore<T>
where
    T: Serialize + for<'de> Deserialize<'de> + Clone + Default + PartialEq + Eq,
{
    /// Create a new session store with the given storage path
    ///
    /// If a storage path is provided, the store will:
    /// - Load existing sessions from disk
    /// - Set up file watching for cross-process synchronization
    /// - Automatically reload when other processes modify the file
    pub fn new(storage_path: Option<PathBuf>) -> Result<Self> {
        let mut store = Self {
            sessions: HashMap::new(),
            storage_path: storage_path.clone(),
            needs_reload: Arc::new(AtomicBool::new(false)),
            ignore_next_events: Arc::new(AtomicUsize::new(0)),
            _watcher: None,
        };

        // Ensure storage directory exists and file is accessible
        if let Some(storage_path) = &storage_path {
            if let Some(parent) = storage_path.parent() {
                fs::create_dir_all(parent)?;
            }

            OpenOptions::new()
                .append(true)
                .create(true)
                .open(storage_path)
                .map_err(|_| anyhow!("could not open {}", storage_path.to_string_lossy()))?;
        }

        // Load existing sessions from disk
        store.load()?;

        // Set up file watching for cross-process synchronization
        if storage_path.is_some() {
            store.setup_file_watching()?;
        }

        Ok(store)
    }

    /// Set up file watching to detect changes from other processes
    fn setup_file_watching(&mut self) -> Result<()> {
        let Some(storage_path) = &self.storage_path else {
            return Ok(());
        };

        let needs_reload = Arc::clone(&self.needs_reload);
        let ignore_next_events = Arc::clone(&self.ignore_next_events);
        let watch_path = storage_path.clone();

        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    log::trace!("received {event:?}");
                    // Reload on content-changing events:
                    // - Modify: direct writes, touch command
                    // - Create: atomic rename completion
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            // Check if we should ignore this event (from our own write)
                            let current = ignore_next_events.load(Ordering::Relaxed);
                            if current > 0 {
                                // Saturating subtraction to prevent underflow
                                let new_value = current.saturating_sub(1);
                                ignore_next_events.store(new_value, Ordering::Relaxed);
                                log::trace!(
                                    "ignoring event from our own write (remaining: {new_value})"
                                );
                                return; // Skip this event - it's from our own write
                            }

                            log::trace!("marking needs_reload");
                            needs_reload.store(true, Ordering::Relaxed);
                        }
                        _ => {} // Ignore access time, metadata changes, etc.
                    }
                }
            },
            notify::Config::default(),
        )?;

        // Watch the specific file for changes
        watcher.watch(&watch_path, RecursiveMode::NonRecursive)?;

        // Store the watcher to keep the background thread alive
        self._watcher = Some(watcher);

        log::trace!("watching {}", watch_path.display());

        Ok(())
    }

    /// Check if we need to reload from disk and do so if necessary
    fn check_and_reload(&mut self) -> Result<()> {
        if self.needs_reload.load(Ordering::Relaxed) {
            log::trace!("needs reload detected");

            self.load()?;
            self.needs_reload.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Get session data, creating a new session if it doesn't exist
    ///
    /// This automatically checks for file changes from other processes before returning data.
    pub fn get_or_create(&mut self, session_id: &str) -> Result<&T> {
        self.check_and_reload()?;

        let mut changed = false;

        // Create or update the entry
        {
            self.sessions
                .entry(session_id.to_string())
                .and_modify(|_e| {
                    // Just reading - no timestamp update, no changes
                    changed = false;
                })
                .or_insert_with(|| {
                    changed = true; // New entry is always a change
                    SessionEntry::default()
                });
        }

        if changed {
            self.save()?;
        }

        Ok(&self.sessions.get(session_id).unwrap().data)
    }

    /// Get immutable reference to session data without creating a new session
    ///
    /// Returns None if the session doesn't exist. This automatically checks for
    /// file changes from other processes.
    pub fn get(&mut self, session_id: &str) -> Result<Option<&T>> {
        self.check_and_reload()?;
        Ok(self.sessions.get(session_id).map(|entry| &entry.data))
    }

    /// Update session data using a closure
    ///
    /// The closure receives a mutable reference to the session data and can modify it.
    /// If the session doesn't exist, it will be created with default values first.
    pub fn update(&mut self, session_id: &str, fun: impl FnOnce(&mut T)) -> Result<()> {
        self.check_and_reload()?;

        let mut changed = false;

        {
            match self.sessions.entry(session_id.to_string()) {
                Entry::Occupied(mut entry) => {
                    let entry = entry.get_mut();
                    let before_data = entry.data.clone();
                    fun(&mut entry.data);
                    if before_data != entry.data {
                        entry.update_last_used();
                        changed = true;
                    }
                }

                Entry::Vacant(vacant) => {
                    let mut entry = SessionEntry::default();
                    fun(&mut entry.data);
                    entry.update_last_used();
                    changed = true; // New entry is always a change
                    vacant.insert(entry);
                }
            }
        }

        if changed {
            self.save()?;
        }
        Ok(())
    }

    /// Set session data directly
    pub fn set(&mut self, session_id: &str, data: T) -> Result<()> {
        self.update(session_id, |existing| *existing = data)
    }

    /// Load sessions from disk
    fn load(&mut self) -> Result<()> {
        if let Some(storage_path) = &self.storage_path {
            if storage_path.exists() {
                log::trace!("reloading {}...", storage_path.display());

                let contents = std::fs::read_to_string(storage_path)?;
                if !contents.trim().is_empty() {
                    if let Ok(sessions) = serde_json::from_str(&contents) {
                        log::debug!("reloaded {}", storage_path.display());

                        self.sessions = sessions;
                    }
                }
            }
        }
        Ok(())
    }

    /// Save sessions to disk using atomic write (temp file + rename)
    fn save(&self) -> Result<()> {
        if let Some(storage_path) = &self.storage_path {
            // TODO: Consider using notify-debouncer-mini for cleaner event handling
            // Expect 2 events from atomic write (empirically observed on macOS)
            self.ignore_next_events.store(2, Ordering::Relaxed);

            log::trace!("saving");
            let temp_path = storage_path.with_extension("tmp");

            let contents = serde_json::to_string_pretty(&self.sessions)?;
            std::fs::write(&temp_path, &contents)?;
            std::fs::rename(temp_path, storage_path)?;
            log::trace!("saved");
        }
        Ok(())
    }
}
