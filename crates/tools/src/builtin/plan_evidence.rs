//! The todo list as evidence: which step a receipt is about, what a serial
//! list allows next, and how a citation resolves to a task-list item.
//!
//! Parity: `internal/safety/evidence/todos.go`, `todo_identity.go`,
//! `todo_progress.go`. These are pure functions of the list they are handed;
//! ledger receipts, baseline continuity and completion transitions stay with
//! the host (`plan.rs`).

#![forbid(unsafe_code)]

use serde::Deserialize;

/// The todo list as evidence (Go `evidence.TodoItem`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct TodoItem {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub status: String,
    #[serde(rename = "activeForm", default)]
    pub active_form: String,
    #[serde(default)]
    pub level: i64,
    #[serde(rename = "step_id", default)]
    pub step_id: String,
}

/// A positive resolution of a citation to a task-list item (Go
/// `evidence.TodoStepMatch`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TodoStepMatch {
    pub found: bool,
    pub index: usize,
    pub content: String,
    pub status: String,
    pub active_form: String,
    pub step_id: String,
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    head: usize,
    end: usize,
}

/// `completed` / `in_progress` / `pending`; an empty status normalizes to
/// `pending` (Go `todoStatus`).
pub fn todo_status(status: &str) -> &str {
    let trimmed = status.trim();
    if trimmed.is_empty() {
        "pending"
    } else {
        trimmed
    }
}

/// Fold the drift models introduce when citing a todo: fullwidth ASCII forms
/// become halfwidth, all whitespace is dropped, matching is case-insensitive
/// (Go `normalizeStepText`).
pub fn normalize_step_text(s: &str) -> String {
    let folded: String = s
        .chars()
        .map(|r| {
            if ('\u{FF01}'..='\u{FF5E}').contains(&r) {
                char::from_u32(r as u32 - 0xFEE0).unwrap_or(r)
            } else {
                r
            }
        })
        .collect();
    let joined: String = folded.split_whitespace().collect();
    joined.to_lowercase()
}

fn same_step_text(a: &str, b: &str) -> bool {
    let na = normalize_step_text(a);
    let nb = normalize_step_text(b);
    !na.is_empty() && na == nb
}

/// Substring match between normalized texts, but only when the shorter side
/// is substantial enough (≥6 runes) to not match by accident (Go
/// `stepTextContains`).
pub fn step_text_contains(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let short = if b.chars().count() < a.chars().count() {
        b
    } else {
        a
    };
    if short.chars().count() < 6 {
        return false;
    }
    a.contains(b) || b.contains(a)
}

fn parse_step_index(step: &str) -> (i64, bool) {
    let trimmed = step.trim().trim_end_matches('.').trim();
    match trimmed.parse::<i64>() {
        Ok(n) => (n, true),
        Err(_) => (0, false),
    }
}

fn text_overlaps(a: &str, b: &str) -> bool {
    step_text_contains(&normalize_step_text(a), &normalize_step_text(b))
}

/// A level-0 header plus its level-1 sub-steps, or a single plain step. `end`
/// is exclusive.
fn serial_todo_segments(todos: &[TodoItem]) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut i = 0;
    while i < todos.len() {
        let mut end = i + 1;
        if todos[i].level == 0 {
            while end < todos.len() && todos[end].level == 1 {
                end += 1;
            }
        }
        segs.push(Segment { head: i, end });
        i = end;
    }
    segs
}

fn segment_completed(todos: &[TodoItem], seg: Segment) -> bool {
    (seg.head..seg.end).all(|i| todo_status(&todos[i].status) == "completed")
}

/// Validates the task-list state machine promised by `todo_write`: at most one
/// item in progress, completed work forms a serial prefix, pending work follows
/// the current item. Segment-aware for two-level lists (Go `ValidateSerialTodos`).
pub fn validate_serial_todos(todos: &[TodoItem]) -> Result<(), String> {
    let mut ip_seen = false;
    for (i, todo) in todos.iter().enumerate() {
        match todo_status(&todo.status) {
            "completed" | "pending" => {}
            "in_progress" => {
                if ip_seen {
                    return Err(format!(
                        "todo {} {:?} is a second in_progress item; serial task lists allow exactly one current item",
                        i + 1,
                        todo.content
                    ));
                }
                ip_seen = true;
            }
            other => {
                return Err(format!(
                    "todo {} {:?} has invalid status {:?}",
                    i + 1,
                    todo.content,
                    other
                ));
            }
        }
    }

    if !todos.is_empty() && todos[0].level == 1 {
        return Err(format!(
            "todo 1 {:?} is a level-1 sub-step with no phase above it; add a level-0 phase header or use level 0",
            todos[0].content
        ));
    }

    let mut seen_current = false;
    let mut seen_pending = false;
    for seg in serial_todo_segments(todos) {
        let state = validate_serial_segment(todos, seg)?;
        match state.as_str() {
            "completed" => {
                if seen_current || seen_pending {
                    return Err(format!(
                        "todo {} {:?} is completed after unfinished work; serial task lists require completed items to form a prefix",
                        seg.head + 1,
                        todos[seg.head].content
                    ));
                }
            }
            "in_progress" => {
                if seen_pending {
                    let mut ip = seg.head;
                    for (offset, todo) in todos[seg.head..seg.end].iter().enumerate() {
                        if todo_status(&todo.status) == "in_progress" {
                            ip = seg.head + offset;
                            break;
                        }
                    }
                    return Err(format!(
                        "todo {} {:?} is in_progress after pending work; the current item must be the first unfinished item",
                        ip + 1,
                        todos[ip].content
                    ));
                }
                seen_current = true;
            }
            "pending" => seen_pending = true,
            _ => {
                // stale: partially completed with no current item
                if seen_current {
                    let mut first = seg.head;
                    for (offset, todo) in todos[seg.head..seg.end].iter().enumerate() {
                        if todo_status(&todo.status) == "completed" {
                            first = seg.head + offset;
                            break;
                        }
                    }
                    return Err(format!(
                        "todo {} {:?} is completed after unfinished work; serial task lists require completed items to form a prefix",
                        first + 1,
                        todos[first].content
                    ));
                }
                seen_pending = true;
            }
        }
    }

    if !todos.is_empty() && seen_pending && !seen_current {
        return Err("serial task list has pending work but no in_progress item".to_owned());
    }
    Ok(())
}

fn validate_serial_segment(todos: &[TodoItem], seg: Segment) -> Result<String, String> {
    let head = &todos[seg.head];
    let head_status = todo_status(&head.status).to_owned();
    if seg.end == seg.head + 1 {
        return Ok(head_status);
    }

    let mut seen_sub_current = false;
    let mut seen_sub_pending = false;
    let mut completed_subs = 0;
    let mut unfinished: isize = -1;
    for (offset, sub) in todos[(seg.head + 1)..seg.end].iter().enumerate() {
        let i = seg.head + 1 + offset;
        match todo_status(&sub.status) {
            "completed" => {
                if seen_sub_current || seen_sub_pending {
                    return Err(format!(
                        "todo {} {:?} is completed after unfinished work; serial task lists require completed items to form a prefix",
                        i + 1,
                        sub.content
                    ));
                }
                completed_subs += 1;
            }
            "in_progress" => {
                if seen_sub_pending {
                    return Err(format!(
                        "todo {} {:?} is in_progress after pending work; the current item must be the first unfinished item",
                        i + 1,
                        sub.content
                    ));
                }
                seen_sub_current = true;
                if unfinished < 0 {
                    unfinished = i as isize;
                }
            }
            _ => {
                seen_sub_pending = true;
                if unfinished < 0 {
                    unfinished = i as isize;
                }
            }
        }
    }

    match head_status.as_str() {
        "completed" => {
            if unfinished >= 0 {
                let u = unfinished as usize;
                return Err(format!(
                    "phase {} {:?} is completed but sub-step {} {:?} is unfinished; complete every sub-step, then sign the phase off with complete_step",
                    seg.head + 1,
                    head.content,
                    u + 1,
                    todos[u].content
                ));
            }
            Ok("completed".to_owned())
        }
        "in_progress" => {
            if unfinished >= 0 {
                let u = unfinished as usize;
                return Err(format!(
                    "phase {} {:?} cannot be in_progress while sub-step {} {:?} is unfinished; keep the phase pending, finish its sub-steps in order, then mark the phase in_progress to sign it off",
                    seg.head + 1,
                    head.content,
                    u + 1,
                    todos[u].content
                ));
            }
            Ok("in_progress".to_owned())
        }
        _ => {
            if seen_sub_current {
                Ok("in_progress".to_owned())
            } else if completed_subs == 0 {
                Ok("pending".to_owned())
            } else {
                Ok("stale".to_owned())
            }
        }
    }
}

/// Repairs legacy host state that predates [`validate_serial_todos`]: keeps the
/// leading run of fully completed segments, makes the first unfinished segment
/// current, and returns every later segment to pending (Go `NormalizeSerialTodos`).
pub fn normalize_serial_todos(todos: &[TodoItem]) -> Vec<TodoItem> {
    let mut out = todos.to_vec();
    let mut unfinished = false;
    for seg in serial_todo_segments(&out) {
        if !unfinished && segment_completed(&out, seg) {
            continue;
        }
        if unfinished {
            for todo in out[seg.head..seg.end].iter_mut() {
                todo.status = "pending".to_owned();
            }
            continue;
        }
        unfinished = true;
        if seg.end == seg.head + 1 {
            out[seg.head].status = "in_progress".to_owned();
            continue;
        }
        let mut sub_unfinished = false;
        for todo in out[(seg.head + 1)..seg.end].iter_mut() {
            if !sub_unfinished && todo_status(&todo.status) == "completed" {
                continue;
            }
            if !sub_unfinished {
                todo.status = "in_progress".to_owned();
                sub_unfinished = true;
                continue;
            }
            todo.status = "pending".to_owned();
        }
        if sub_unfinished {
            out[seg.head].status = "pending".to_owned();
        } else {
            out[seg.head].status = "in_progress".to_owned();
        }
    }
    out
}

/// Whether `todos[index]` is a level-0 phase with level-1 sub-steps, and if so
/// the 0-based index of its first unfinished sub-step. `ok` is false when the
/// index is not a phase header; an all-completed phase returns `(-1, true)`.
pub fn first_unfinished_sub_step(todos: &[TodoItem], index: isize) -> (isize, bool) {
    let index = index as usize;
    if todos.is_empty() || index >= todos.len() || todos[index].level != 0 {
        return (-1, false);
    }
    if index + 1 >= todos.len() || todos[index + 1].level != 1 {
        return (-1, false);
    }
    for (offset, todo) in todos[(index + 1)..].iter().enumerate() {
        let i = index + 1 + offset;
        if todo.level != 1 {
            break;
        }
        if todo_status(&todo.status) != "completed" {
            return (i as isize, true);
        }
    }
    (-1, true)
}

/// Completes the in_progress item at `index` (0-based) and promotes the next
/// serial item so exactly one item stays current. Reports whether the item was
/// completed (Go `AdvanceSerialTodo`).
pub fn advance_serial_todo(todos: &mut [TodoItem], index: usize) -> bool {
    if index >= todos.len() {
        return false;
    }
    if todo_status(&todos[index].status) != "in_progress" {
        return false;
    }
    if let (unfinished, true) = first_unfinished_sub_step(todos, index as isize)
        && unfinished >= 0
    {
        return false;
    }
    todos[index].status = "completed".to_owned();
    if todos[index].level == 1 {
        for todo in todos[(index + 1)..].iter_mut() {
            if todo.level != 1 {
                break;
            }
            if todo_status(&todo.status) == "pending" {
                todo.status = "in_progress".to_owned();
                return true;
            }
        }
        let mut head = index as isize - 1;
        while head >= 0 && todos[head as usize].level == 1 {
            head -= 1;
        }
        if head >= 0 {
            let head = head as usize;
            if todo_status(&todos[head].status) != "completed" {
                todos[head].status = "in_progress".to_owned();
            }
            return true;
        }
    }
    if todos
        .iter()
        .any(|t| todo_status(&t.status) == "in_progress")
    {
        return true;
    }
    for i in 0..todos.len() {
        if todo_status(&todos[i].status) != "pending" {
            continue;
        }
        if let (sub, true) = first_unfinished_sub_step(todos, i as isize)
            && sub >= 0
            && todo_status(&todos[sub as usize].status) == "pending"
        {
            todos[sub as usize].status = "in_progress".to_owned();
            return true;
        }
        todos[i].status = "in_progress".to_owned();
        return true;
    }
    true
}

/// Items of a todo list that are not completed (Go `IncompleteTodos`).
pub fn incomplete_todos(todos: &[TodoItem]) -> Vec<TodoStepMatch> {
    todos
        .iter()
        .enumerate()
        .filter_map(|(j, t)| {
            let status = todo_status(&t.status);
            (status != "completed").then(|| TodoStepMatch {
                found: true,
                index: j + 1,
                content: t.content.clone(),
                status: status.to_owned(),
                active_form: t.active_form.clone(),
                step_id: t.step_id.clone(),
            })
        })
        .collect()
}

/// Resolves a stable step id against the current list (Go `MatchStepID`).
pub fn match_step_id(step_id: &str, todos: &[TodoItem]) -> (TodoStepMatch, bool) {
    let step_id = step_id.trim();
    if step_id.is_empty() {
        return (TodoStepMatch::default(), false);
    }
    for (i, todo) in todos.iter().enumerate() {
        if todo.step_id == step_id {
            return (todo_match_at(i + 1, todo), true);
        }
    }
    (TodoStepMatch::default(), false)
}

/// Lists the stable ids present in the list, for error messages that name the
/// identities a model may cite (Go `TodoStepIDs`).
pub fn todo_step_ids(todos: &[TodoItem]) -> Vec<String> {
    todos
        .iter()
        .filter(|t| !t.step_id.is_empty())
        .map(|t| t.step_id.clone())
        .collect()
}

/// How one task-list item names itself wherever the model is shown the list
/// (Go `TodoCitation`).
pub fn todo_citation(step_id: &str, index: usize, content: &str) -> String {
    let id = step_id.trim();
    if !id.is_empty() {
        format!("[{id}] {content}")
    } else {
        format!("{index}) {content}")
    }
}

/// The one item a sign-off may currently name (Go `InProgressTodo`).
pub fn in_progress_todo(todos: &[TodoItem]) -> (TodoStepMatch, bool) {
    for (i, todo) in todos.iter().enumerate() {
        if todo.status.trim() == "in_progress" {
            return (todo_match_at(i + 1, todo), true);
        }
    }
    (TodoStepMatch::default(), false)
}

fn todo_match_at(index: usize, todo: &TodoItem) -> TodoStepMatch {
    TodoStepMatch {
        found: true,
        index,
        content: todo.content.clone(),
        status: todo.status.clone(),
        active_form: todo.active_form.clone(),
        step_id: todo.step_id.clone(),
    }
}

fn same_todo_identity(a: &TodoItem, b: &TodoItem) -> bool {
    if !a.step_id.is_empty() && !b.step_id.is_empty() {
        return a.step_id == b.step_id;
    }
    same_step_text(&a.content, &b.content) || same_step_text(&a.active_form, &b.active_form)
}

fn todo_content_relates(todo: &TodoItem, r#match: &TodoStepMatch) -> bool {
    text_overlaps(&todo.content, &r#match.content)
        || text_overlaps(&todo.active_form, &r#match.active_form)
}

/// Resolves an existing todo against an updated list without interpreting
/// numeric content as a 1-based step citation (Go `MatchTodoIdentity`).
pub fn match_todo_identity(todo: &TodoItem, todos: &[TodoItem]) -> (TodoStepMatch, bool) {
    for (i, candidate) in todos.iter().enumerate() {
        if same_todo_identity(todo, candidate) {
            return (todo_match_at(i + 1, candidate), true);
        }
    }
    let mut found: isize = -1;
    for (i, candidate) in todos.iter().enumerate() {
        let match_ = TodoStepMatch {
            content: candidate.content.clone(),
            active_form: candidate.active_form.clone(),
            ..Default::default()
        };
        if !todo_content_relates(todo, &match_) {
            continue;
        }
        if found >= 0 && found as usize != i {
            return (TodoStepMatch::default(), false);
        }
        found = i as isize;
    }
    if found < 0 {
        return (TodoStepMatch::default(), false);
    }
    let found = found as usize;
    (todo_match_at(found + 1, &todos[found]), true)
}

/// Whether every previously completed item remains completed at the same index
/// in the replacement list (Go `PreservesCompletedTodoPositions`).
pub fn preserves_completed_todo_positions(previous: &[TodoItem], next: &[TodoItem]) -> bool {
    for (i, todo) in previous.iter().enumerate() {
        if todo_status(&todo.status) != "completed" {
            continue;
        }
        if i >= next.len() || todo_status(&next[i].status) != "completed" {
            return false;
        }
        let (match_, found) = match_todo_identity(todo, next);
        if !found || match_.index != i + 1 {
            return false;
        }
    }
    true
}

/// Whether two task lists say the same thing, both normalized first (Go
/// `SameTodos`).
pub fn same_todos(a: &[TodoItem], b: &[TodoItem]) -> bool {
    normalize_serial_todos(a) == normalize_serial_todos(b)
}

/// Resolves a complete_step citation (number, title, or drift-tolerant variant)
/// against a todo list (Go `MatchStep`).
pub fn match_step(step: &str, todos: &[TodoItem]) -> (TodoStepMatch, bool) {
    let m = match_todo_step(step, todos);
    let found = m.found;
    (m, found)
}

fn match_todo_step(step: &str, todos: &[TodoItem]) -> TodoStepMatch {
    let (m, ok) = match_step_id(step, todos);
    if ok {
        return m;
    }
    if let (n, true) = parse_step_index(&normalize_step_text(step))
        && n >= 1
        && (n as usize) <= todos.len()
    {
        let t = &todos[n as usize - 1];
        return todo_match_at(n as usize, t);
    }
    for (i, t) in todos.iter().enumerate() {
        if same_step_text(step, &t.content) || same_step_text(step, &t.active_form) {
            return todo_match_at(i + 1, t);
        }
    }
    // Containment fallback for wording drift; an ambiguous citation (containing
    // or contained by two different todos) stays unmatched rather than guessing.
    let norm = normalize_step_text(step);
    let mut found: isize = -1;
    for (i, t) in todos.iter().enumerate() {
        if step_text_contains(&norm, &normalize_step_text(&t.content))
            || step_text_contains(&norm, &normalize_step_text(&t.active_form))
        {
            if found >= 0 && found as usize != i {
                return TodoStepMatch::default();
            }
            found = i as isize;
        }
    }
    if found >= 0 {
        let found = found as usize;
        return todo_match_at(found + 1, &todos[found]);
    }
    TodoStepMatch::default()
}
