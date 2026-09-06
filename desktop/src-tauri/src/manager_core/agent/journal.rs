use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Deserialize;
use uuid::Uuid;

use super::detect::read_config;
use super::files::{
    create_private_backup, remove_and_sync, remove_target_and_sync, replacement_occurred,
    restrict_private, write_atomic,
};
use super::modelconfig::TokenSigner;
use super::plan::{
    result_for_plan, zero_bytes, FileRevision, JournalEntry, JournalProgress, JournalScope,
    Operation, PlannedFile, PlannedRevisionGuard, TransactionJournal, WritePlan, WriteResult,
    REVISION_CONTEXT_BACKUP, REVISION_CONTEXT_JOURNAL,
};
use super::render::RenderInput;
use super::revision::{
    content_revision, keyed_revision_for_content, read_keyed_revision, read_plain_revision,
    revision_context_for, valid_journal_revision, verify_keyed_revision,
};
use super::types::OperationError;
use crate::protocol::ErrorCode;

pub const JOURNAL_FILE_NAME: &str = "agent-write-journal.json";

pub fn journal_path(state_dir: &Path) -> PathBuf {
    state_dir.join(JOURNAL_FILE_NAME)
}

pub fn journal_present(state_dir: &Path) -> bool {
    journal_path(state_dir).exists()
}

pub fn prepare_state_dir(state_dir: &Path) -> Result<(), OperationError> {
    std::fs::create_dir_all(state_dir).map_err(|_| write_failed_state())?;
    restrict_private(state_dir, true).map_err(|_| write_failed_state())?;
    Ok(())
}

pub fn execute_plan(
    signer: &TokenSigner,
    state_dir: &Path,
    key_generation: &str,
    plan: &mut WritePlan,
    key: &str,
    deadline: Option<Instant>,
) -> Result<WriteResult, OperationError> {
    prepare_state_dir(state_dir)?;
    let transaction_id = random_id();
    let mut result = result_for_plan(transaction_id.clone(), plan);
    let mut journal = TransactionJournal {
        version: 3,
        key_generation: key_generation.to_owned(),
        transaction_id,
        committed: false,
        entries: Vec::with_capacity(plan.files.len()),
    };
    let mut created_backups = Vec::new();
    for file in &mut plan.files {
        if deadline_exceeded(deadline) {
            return pre_mutation_failure(state_dir, &mut result, &created_backups, timeout());
        }
        if file.backup_required {
            if verify_planned_revision(
                signer,
                file.source_path.as_ref(),
                &file.source_revision,
                file.scope,
            )
            .is_err()
            {
                return pre_mutation_failure(
                    state_dir,
                    &mut result,
                    &created_backups,
                    OperationError::new(
                        ErrorCode::PreviewStale,
                        "Agent configuration changed before backup",
                    ),
                );
            }
            if file.source_path != file.target_path
                && verify_planned_revision(
                    signer,
                    file.target_path.as_ref(),
                    &file.target_revision,
                    file.scope,
                )
                .is_err()
            {
                return pre_mutation_failure(
                    state_dir,
                    &mut result,
                    &created_backups,
                    OperationError::new(
                        ErrorCode::PreviewStale,
                        "Agent configuration target changed before backup",
                    ),
                );
            }
            match create_private_backup(
                Path::new(&file.backup_source),
                &file.source_content,
                file.source_mode,
                "bak",
            ) {
                Ok(path) => {
                    file.backup_path = path.to_string_lossy().into_owned();
                    created_backups.push(file.backup_path.clone());
                    if file.scope == JournalScope::ManagerState {
                        result.state_backup = Some(super::plan::FileWriteStatus {
                            path: file.backup_path.clone(),
                            operation: Operation::Backup,
                            backup_path: file.backup_path.clone(),
                            ..super::plan::FileWriteStatus::default()
                        });
                    } else if let Some(agent) = file.agent {
                        append_agent_backup(
                            &mut result,
                            agent,
                            &file.target_path,
                            &file.backup_path,
                        );
                    }
                }
                Err(_) => {
                    return backup_failure(state_dir, &mut result, &created_backups);
                }
            }
        }
        let mut output = match file.output(&plan.input, key) {
            Ok(output) => output,
            Err(_) => {
                return pre_mutation_failure(
                    state_dir,
                    &mut result,
                    &created_backups,
                    OperationError::new(
                        ErrorCode::WriteFailed,
                        "could not render an Agent configuration file",
                    ),
                );
            }
        };
        let post_revision = if file.operation != Operation::Delete {
            let mode = if file.target_revision.exists {
                file.target_mode
            } else {
                0o600
            };
            match keyed_revision_for_content(
                signer,
                &output,
                mode,
                REVISION_CONTEXT_JOURNAL,
                &file.target_path,
            ) {
                Ok(revision) => revision,
                Err(_) => {
                    zero_bytes(&mut output);
                    return pre_mutation_failure(
                        state_dir,
                        &mut result,
                        &created_backups,
                        OperationError::new(
                            ErrorCode::ModelStateInvalid,
                            "Agent revision state could not be created",
                        ),
                    );
                }
            }
        } else {
            FileRevision::default()
        };
        let (pre_revision, _, _) = match read_keyed_revision(
            signer,
            Path::new(&file.target_path),
            REVISION_CONTEXT_JOURNAL,
        ) {
            Ok(value) => value,
            Err(_) => {
                zero_bytes(&mut output);
                return pre_mutation_failure(
                    state_dir,
                    &mut result,
                    &created_backups,
                    OperationError::new(
                        ErrorCode::WriteFailed,
                        "could not snapshot Agent transaction state",
                    ),
                );
            }
        };
        zero_bytes(&mut output);
        let backup_revision = if !file.backup_path.is_empty() {
            match keyed_revision_for_content(
                signer,
                &file.source_content,
                file.source_mode,
                REVISION_CONTEXT_BACKUP,
                &file.backup_path,
            ) {
                Ok(revision) => revision,
                Err(_) => {
                    return pre_mutation_failure(
                        state_dir,
                        &mut result,
                        &created_backups,
                        OperationError::new(
                            ErrorCode::ModelStateInvalid,
                            "Agent backup revision could not be created",
                        ),
                    );
                }
            }
        } else {
            FileRevision::default()
        };
        let journal_operation = if file.operation == Operation::Delete {
            Operation::Delete
        } else {
            Operation::Replace
        };
        journal.entries.push(JournalEntry {
            scope: Some(file.scope),
            agent: file.agent,
            target_path: file.target_path.clone(),
            operation: Some(journal_operation),
            pre_revision,
            post_revision,
            backup_path: file.backup_path.clone(),
            backup_revision,
            restore_from: file.restore_from.clone(),
            rollback_backup_path: String::new(),
            target_mode: file.target_mode & 0o777,
            progress: JournalProgress::Pending,
        });
    }
    if deadline_exceeded(deadline) {
        return pre_mutation_failure(state_dir, &mut result, &created_backups, timeout());
    }
    if verify_plan_revision_set(signer, plan).is_err() {
        return pre_mutation_failure(
            state_dir,
            &mut result,
            &created_backups,
            OperationError::new(
                ErrorCode::PreviewStale,
                "Agent configuration changed before transaction journal",
            ),
        );
    }
    if write_journal(state_dir, &journal).is_err() {
        let failure = OperationError::new(
            ErrorCode::WriteFailed,
            "could not persist Agent transaction journal",
        );
        if remove_and_sync(&journal_path(state_dir)).is_err() {
            mark_result_failure(&mut result, ErrorCode::RollbackFailed);
            return Err(recovery_failed());
        }
        return pre_mutation_failure(state_dir, &mut result, &created_backups, failure);
    }

    let mut replaced = false;
    for (index, file) in plan.files.iter().enumerate() {
        if deadline_exceeded(deadline) {
            return fail_and_rollback(signer, state_dir, &mut journal, result, timeout(), replaced);
        }
        if verify_planned_revision(
            signer,
            file.source_path.as_ref(),
            &file.source_revision,
            file.scope,
        )
        .is_err()
        {
            return fail_and_rollback(
                signer,
                state_dir,
                &mut journal,
                result,
                OperationError::new(
                    ErrorCode::PreviewStale,
                    "Agent configuration changed while writing",
                ),
                replaced,
            );
        }
        if file.source_path != file.target_path
            && verify_planned_revision(
                signer,
                file.target_path.as_ref(),
                &file.target_revision,
                file.scope,
            )
            .is_err()
        {
            return fail_and_rollback(
                signer,
                state_dir,
                &mut journal,
                result,
                OperationError::new(
                    ErrorCode::PreviewStale,
                    "Agent configuration target changed while writing",
                ),
                replaced,
            );
        }
        if verify_revision_guards(signer, &plan.revision_guards).is_err() {
            return fail_and_rollback(
                signer,
                state_dir,
                &mut journal,
                result,
                OperationError::new(
                    ErrorCode::PreviewStale,
                    "Agent configuration changed while writing",
                ),
                replaced,
            );
        }
        match apply_planned_file(file, &plan.input, key) {
            Ok(mut output) => {
                zero_bytes(&mut output);
                replaced = true;
            }
            Err(error) => {
                if replacement_occurred(error.as_ref()) {
                    replaced = true;
                    mark_planned_file_replaced(&mut result, file);
                }
                return fail_and_rollback(
                    signer,
                    state_dir,
                    &mut journal,
                    result,
                    OperationError::new(
                        ErrorCode::WriteFailed,
                        "could not apply an Agent configuration file change",
                    ),
                    replaced,
                );
            }
        }
        if verify_journal_revision(
            Some(signer),
            3,
            Path::new(&file.target_path),
            &journal.entries[index].post_revision,
        )
        .is_err()
        {
            return fail_and_rollback(
                signer,
                state_dir,
                &mut journal,
                result,
                OperationError::new(
                    ErrorCode::WriteFailed,
                    "could not verify an Agent configuration file change",
                ),
                true,
            );
        }
        mark_planned_file_replaced(&mut result, file);
        journal.entries[index].progress = JournalProgress::Replaced;
        if write_journal(state_dir, &journal).is_err() {
            return fail_and_rollback(
                signer,
                state_dir,
                &mut journal,
                result,
                OperationError::new(
                    ErrorCode::WriteFailed,
                    "could not record Agent transaction progress",
                ),
                true,
            );
        }
    }
    if deadline_exceeded(deadline) {
        return fail_and_rollback(signer, state_dir, &mut journal, result, timeout(), replaced);
    }
    if verify_revision_guards(signer, &plan.revision_guards).is_err() {
        return fail_and_rollback(
            signer,
            state_dir,
            &mut journal,
            result,
            OperationError::new(
                ErrorCode::PreviewStale,
                "Agent configuration changed before transaction commit",
            ),
            replaced,
        );
    }
    journal.committed = true;
    if write_journal(state_dir, &journal).is_err() {
        return fail_and_rollback(
            signer,
            state_dir,
            &mut journal,
            result,
            OperationError::new(
                ErrorCode::WriteFailed,
                "could not commit Agent transaction journal",
            ),
            replaced,
        );
    }
    let _ = remove_and_sync(&journal_path(state_dir));
    for agent in &mut result.agents {
        agent.success = true;
    }
    Ok(result)
}

fn apply_planned_file(
    file: &PlannedFile,
    input: &RenderInput,
    key: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    if file.operation == Operation::Delete {
        remove_target_and_sync(Path::new(&file.target_path))?;
        return Ok(Vec::new());
    }
    let output = file
        .output(input, key)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    let mode = if file.target_revision.exists {
        file.target_mode
    } else {
        0o600
    };
    write_atomic(
        Path::new(&file.target_path),
        &output,
        mode,
        !file.target_revision.exists,
    )?;
    Ok(output)
}

pub fn recover_locked(
    signer: Option<&TokenSigner>,
    key_generation: &str,
    state_dir: &Path,
) -> Result<(), OperationError> {
    let path = journal_path(state_dir);
    match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(recovery_failed()),
        Ok(_) => {}
    }
    let mut journal = decode_journal(&path)?;
    if journal.version >= 2 {
        let signer = signer.ok_or_else(recovery_failed)?;
        if journal.key_generation != key_generation {
            return Err(recovery_failed());
        }
        if journal.committed {
            return remove_and_sync(&path).map_err(|_| recovery_failed());
        }
        for entry in &journal.entries {
            if !entry.backup_path.is_empty() {
                restrict_private(Path::new(&entry.backup_path), false)
                    .map_err(|_| recovery_failed())?;
            }
            if !entry.rollback_backup_path.is_empty() {
                restrict_private(Path::new(&entry.rollback_backup_path), false)
                    .map_err(|_| recovery_failed())?;
            }
        }
        rollback(Some(signer), state_dir, &mut journal, None)
    } else {
        if journal.committed {
            return remove_and_sync(&path).map_err(|_| recovery_failed());
        }
        rollback(None, state_dir, &mut journal, None)
    }
}

fn fail_and_rollback(
    signer: &TokenSigner,
    state_dir: &Path,
    journal: &mut TransactionJournal,
    mut result: WriteResult,
    original: OperationError,
    replaced: bool,
) -> Result<WriteResult, OperationError> {
    if !replaced {
        if remove_and_sync(&journal_path(state_dir)).is_err() {
            mark_result_failure(&mut result, ErrorCode::RollbackFailed);
            return Err(recovery_failed());
        }
        mark_result_failure(&mut result, original.code);
        return Err(original);
    }
    if rollback(Some(signer), state_dir, journal, Some(&mut result)).is_err() {
        mark_result_failure(&mut result, ErrorCode::RollbackFailed);
        return Err(recovery_failed());
    }
    mark_result_failure(&mut result, original.code);
    for agent in &mut result.agents {
        if agent.files.iter().any(|file| file.restored) {
            agent.rolled_back = true;
        }
    }
    Err(original)
}

fn rollback(
    signer: Option<&TokenSigner>,
    state_dir: &Path,
    journal: &mut TransactionJournal,
    mut result: Option<&mut WriteResult>,
) -> Result<(), OperationError> {
    let mut conflict = false;
    for index in (0..journal.entries.len()).rev() {
        let target = journal.entries[index].target_path.clone();
        let pre = journal.entries[index].pre_revision.clone();
        let post = journal.entries[index].post_revision.clone();
        if verify_journal_revision(signer, journal.version, Path::new(&target), &pre).is_ok() {
            journal.entries[index].progress = JournalProgress::Restored;
            write_journal(state_dir, journal)?;
            continue;
        }
        if verify_journal_revision(signer, journal.version, Path::new(&target), &post).is_err() {
            conflict = true;
            continue;
        }
        let (current_revision, mut current_content, current_mode) =
            read_journal_revision(signer, journal.version, Path::new(&target))?;
        if current_revision.exists && journal.entries[index].rollback_backup_path.is_empty() {
            let backup = create_private_backup(
                Path::new(&target),
                &current_content,
                current_mode,
                &format!("rollback-{}", journal.transaction_id),
            )
            .map_err(|_| recovery_failed())?;
            zero_bytes(&mut current_content);
            journal.entries[index].rollback_backup_path = backup.to_string_lossy().into_owned();
            if let Some(result) = result.as_deref_mut() {
                if journal.entries[index].scope != Some(JournalScope::ManagerState) {
                    if let Some(agent) = journal.entries[index].agent {
                        append_rollback_backup(
                            result,
                            agent,
                            &target,
                            &journal.entries[index].rollback_backup_path,
                        );
                    }
                } else if let Some(state) = result.state_change.as_mut() {
                    state.rollback_backup_path =
                        journal.entries[index].rollback_backup_path.clone();
                }
            }
            write_journal(state_dir, journal)?;
        }
        if journal.entries[index].pre_revision.exists {
            let mut backup_content = read_config(Path::new(&journal.entries[index].backup_path))
                .map_err(|_| recovery_failed())?;
            if verify_backup_content(
                signer,
                journal.version,
                &journal.entries[index].backup_path,
                &backup_content,
                &journal.entries[index].pre_revision,
                &journal.entries[index].backup_revision,
            )
            .is_err()
            {
                zero_bytes(&mut backup_content);
                return Err(recovery_failed());
            }
            write_atomic(
                Path::new(&target),
                &backup_content,
                journal.entries[index].target_mode,
                false,
            )
            .map_err(|_| recovery_failed())?;
            zero_bytes(&mut backup_content);
        } else {
            remove_and_sync(Path::new(&target)).map_err(|_| recovery_failed())?;
        }
        verify_journal_revision(
            signer,
            journal.version,
            Path::new(&target),
            &journal.entries[index].pre_revision,
        )?;
        journal.entries[index].progress = JournalProgress::Restored;
        if let Some(result) = result.as_deref_mut() {
            if journal.entries[index].scope != Some(JournalScope::ManagerState) {
                if let Some(agent) = journal.entries[index].agent {
                    mark_file_restored(result, agent, &target);
                }
            } else if let Some(state) = result.state_change.as_mut() {
                state.restored = true;
            }
        }
        write_journal(state_dir, journal)?;
    }
    if conflict {
        return Err(recovery_failed());
    }
    for entry in &journal.entries {
        verify_journal_revision(
            signer,
            journal.version,
            Path::new(&entry.target_path),
            &entry.pre_revision,
        )?;
    }
    remove_and_sync(&journal_path(state_dir)).map_err(|_| recovery_failed())
}

fn write_journal(state_dir: &Path, journal: &TransactionJournal) -> Result<(), OperationError> {
    let mut content = serde_json::to_vec_pretty(journal).map_err(|_| write_failed_state())?;
    content.push(b'\n');
    let result = write_atomic(&journal_path(state_dir), &content, 0o600, true)
        .map_err(|_| write_failed_state());
    zero_bytes(&mut content);
    result
}

fn decode_journal(path: &Path) -> Result<TransactionJournal, OperationError> {
    let content = read_config(path).map_err(|_| recovery_failed())?;
    let mut deserializer = serde_json::Deserializer::from_slice(&content);
    let mut journal =
        TransactionJournal::deserialize(&mut deserializer).map_err(|_| recovery_failed())?;
    deserializer.end().map_err(|_| recovery_failed())?;
    if (journal.version != 1 && journal.version != 2 && journal.version != 3)
        || journal.transaction_id.is_empty()
        || journal.entries.is_empty()
    {
        return Err(recovery_failed());
    }
    if journal.version >= 2 && journal.key_generation.is_empty() {
        return Err(recovery_failed());
    }
    let last = journal.entries.len() - 1;
    for (index, entry) in journal.entries.iter_mut().enumerate() {
        if journal.version < 3 {
            if entry.operation.is_some() {
                return Err(recovery_failed());
            }
            entry.operation = Some(Operation::Replace);
        }
        if entry.target_path.is_empty()
            || !Path::new(&entry.target_path).is_absolute()
            || (entry.scope != Some(JournalScope::ManagerState) && entry.agent.is_none())
            || (entry.scope == Some(JournalScope::ManagerState) && entry.agent.is_some())
            || (journal.version == 3
                && entry.scope != Some(JournalScope::Agent)
                && entry.scope != Some(JournalScope::ManagerState))
            || (entry.scope == Some(JournalScope::ManagerState) && index != last)
        {
            return Err(recovery_failed());
        }
        if !valid_journal_revision(&entry.pre_revision)
            || !valid_journal_revision(&entry.post_revision)
            || !valid_journal_revision(&entry.backup_revision)
        {
            return Err(recovery_failed());
        }
        if entry.pre_revision.exists && entry.backup_path.is_empty() {
            return Err(recovery_failed());
        }
        if journal.version >= 2 && entry.pre_revision.exists && !entry.backup_revision.exists {
            return Err(recovery_failed());
        }
        let operation = entry.operation.unwrap_or(Operation::Replace);
        if (operation != Operation::Replace && operation != Operation::Delete)
            || (operation == Operation::Replace && !entry.post_revision.exists)
            || (operation == Operation::Delete
                && (entry.post_revision != FileRevision::default() || !entry.pre_revision.exists))
        {
            return Err(recovery_failed());
        }
        if !entry.backup_path.is_empty() {
            let backup_dir = Path::new(&entry.backup_path).parent();
            let restore_dir = Path::new(&entry.restore_from).parent();
            let target_dir = Path::new(&entry.target_path).parent();
            if backup_dir != restore_dir || backup_dir != target_dir {
                return Err(recovery_failed());
            }
        }
    }
    Ok(journal)
}

fn verify_planned_revision(
    signer: &TokenSigner,
    path: &Path,
    expected: &FileRevision,
    scope: JournalScope,
) -> Result<(), OperationError> {
    verify_keyed_revision(signer, path, expected, revision_context_for(scope))
}

fn verify_plan_revision_set(signer: &TokenSigner, plan: &WritePlan) -> Result<(), OperationError> {
    let mut seen: HashMap<(String, JournalScope), FileRevision> = HashMap::new();
    let mut verify = |path: &str, expected: &FileRevision, scope: JournalScope| {
        let identity = (super::plan::clean_path(path), scope);
        if let Some(previous) = seen.get(&identity) {
            if previous != expected {
                return Err(revision_mismatch());
            }
            return Ok(());
        }
        seen.insert(identity, expected.clone());
        verify_planned_revision(signer, Path::new(path), expected, scope)
    };
    for file in &plan.files {
        verify(&file.source_path, &file.source_revision, file.scope)?;
        if file.source_path != file.target_path {
            verify(&file.target_path, &file.target_revision, file.scope)?;
        }
    }
    for guard in &plan.revision_guards {
        verify(&guard.path, &guard.revision, guard.scope)?;
    }
    Ok(())
}

fn verify_revision_guards(
    signer: &TokenSigner,
    guards: &[PlannedRevisionGuard],
) -> Result<(), OperationError> {
    for guard in guards {
        verify_planned_revision(signer, Path::new(&guard.path), &guard.revision, guard.scope)?;
    }
    Ok(())
}

fn verify_journal_revision(
    signer: Option<&TokenSigner>,
    version: i64,
    path: &Path,
    expected: &FileRevision,
) -> Result<(), OperationError> {
    if version == 1 {
        let (current, _, _) = read_plain_revision(path)?;
        if &current != expected {
            return Err(revision_mismatch());
        }
        return Ok(());
    }
    let signer = signer.ok_or_else(recovery_failed)?;
    verify_keyed_revision(signer, path, expected, REVISION_CONTEXT_JOURNAL)
}

fn read_journal_revision(
    signer: Option<&TokenSigner>,
    version: i64,
    path: &Path,
) -> Result<(FileRevision, Vec<u8>, u32), OperationError> {
    if version == 1 {
        return read_plain_revision(path);
    }
    let signer = signer.ok_or_else(recovery_failed)?;
    read_keyed_revision(signer, path, REVISION_CONTEXT_JOURNAL)
}

fn verify_backup_content(
    signer: Option<&TokenSigner>,
    version: i64,
    path: &str,
    content: &[u8],
    expected: &FileRevision,
    backup_expected: &FileRevision,
) -> Result<(), OperationError> {
    if content.len() as i64 != expected.size {
        return Err(recovery_failed());
    }
    if version == 1 {
        let revision = content_revision(content, expected.mode);
        if revision.digest != expected.digest {
            return Err(recovery_failed());
        }
        return Ok(());
    }
    let signer = signer.ok_or_else(recovery_failed)?;
    let revision = keyed_revision_for_content(
        signer,
        content,
        backup_expected.mode,
        REVISION_CONTEXT_BACKUP,
        path,
    )?;
    if revision != *backup_expected {
        return Err(recovery_failed());
    }
    Ok(())
}

fn pre_mutation_failure(
    _state_dir: &Path,
    result: &mut WriteResult,
    created_backups: &[String],
    failure: OperationError,
) -> Result<WriteResult, OperationError> {
    match cleanup_created_backups(created_backups) {
        Ok(()) => {
            clear_result_backups(result);
            mark_result_failure(result, failure.code);
            Err(failure)
        }
        Err(_) => {
            retain_result_backups(result, created_backups);
            mark_result_failure(result, ErrorCode::RollbackFailed);
            Err(recovery_failed())
        }
    }
}

fn backup_failure(
    state_dir: &Path,
    result: &mut WriteResult,
    created_backups: &[String],
) -> Result<WriteResult, OperationError> {
    pre_mutation_failure(
        state_dir,
        result,
        created_backups,
        OperationError::new(
            ErrorCode::BackupFailed,
            "could not create an Agent configuration backup",
        ),
    )
}

fn cleanup_created_backups(paths: &[String]) -> Result<(), OperationError> {
    for path in paths.iter().rev() {
        remove_and_sync(Path::new(path)).map_err(|_| recovery_failed())?;
    }
    Ok(())
}

fn clear_result_backups(result: &mut WriteResult) {
    result.state_backup = None;
    for agent in &mut result.agents {
        agent.backups.clear();
        for file in &mut agent.files {
            file.backup_path.clear();
        }
    }
}

fn retain_result_backups(result: &mut WriteResult, remaining: &[String]) {
    let keep: HashMap<_, _> = remaining.iter().map(|path| (path.as_str(), true)).collect();
    if result
        .state_backup
        .as_ref()
        .is_some_and(|value| !keep.contains_key(value.backup_path.as_str()))
    {
        result.state_backup = None;
    }
    for agent in &mut result.agents {
        agent
            .backups
            .retain(|path| keep.contains_key(path.as_str()));
        for file in &mut agent.files {
            if !keep.contains_key(file.backup_path.as_str()) {
                file.backup_path.clear();
            }
        }
    }
}

fn append_agent_backup(
    result: &mut WriteResult,
    kind: super::modelconfig::Agent,
    target_path: &str,
    backup_path: &str,
) {
    if let Some(agent) = result.agents.iter_mut().find(|agent| agent.agent == kind) {
        agent.backups.push(backup_path.to_owned());
        if let Some(file) = agent.files.iter_mut().find(|file| file.path == target_path) {
            file.backup_path = backup_path.to_owned();
        }
    }
}

fn append_rollback_backup(
    result: &mut WriteResult,
    kind: super::modelconfig::Agent,
    target_path: &str,
    backup_path: &str,
) {
    if let Some(agent) = result.agents.iter_mut().find(|agent| agent.agent == kind) {
        agent.rollback_backups.push(backup_path.to_owned());
        if let Some(file) = agent.files.iter_mut().find(|file| file.path == target_path) {
            file.rollback_backup_path = backup_path.to_owned();
        }
    }
}

fn mark_planned_file_replaced(result: &mut WriteResult, file: &PlannedFile) {
    if file.scope == JournalScope::ManagerState {
        if let Some(state) = result.state_change.as_mut() {
            state.operation = file.operation;
            state.replaced = true;
        }
        return;
    }
    if let Some(agent) = file.agent {
        if let Some(status) = result.agents.iter_mut().find(|item| item.agent == agent) {
            status.changed.push(file.target_path.clone());
            if let Some(item) = status
                .files
                .iter_mut()
                .find(|item| item.path == file.target_path)
            {
                item.replaced = true;
            }
        }
    }
}

fn mark_file_restored(result: &mut WriteResult, kind: super::modelconfig::Agent, path: &str) {
    if let Some(agent) = result.agents.iter_mut().find(|agent| agent.agent == kind) {
        if let Some(file) = agent.files.iter_mut().find(|file| file.path == path) {
            file.restored = true;
        }
    }
}

fn mark_result_failure(result: &mut WriteResult, code: ErrorCode) {
    for agent in &mut result.agents {
        agent.success = false;
        agent.error_code = Some(code);
    }
}

fn random_id() -> String {
    Uuid::new_v4()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn deadline_exceeded(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|value| Instant::now() >= value)
}

fn timeout() -> OperationError {
    OperationError::new(
        ErrorCode::OperationTimeout,
        "Agent operation deadline exceeded",
    )
}

fn write_failed_state() -> OperationError {
    OperationError::new(
        ErrorCode::WriteFailed,
        "could not prepare Agent transaction state",
    )
}

fn recovery_failed() -> OperationError {
    OperationError::new(
        ErrorCode::RollbackFailed,
        "Agent transaction recovery could not prove restoration",
    )
}

fn revision_mismatch() -> OperationError {
    OperationError::new(
        ErrorCode::PreviewStale,
        "Agent configuration changed after preview",
    )
}
