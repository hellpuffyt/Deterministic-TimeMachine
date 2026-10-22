//! `tm inspect`, `tm diff`, `tm validate`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use timemachine::Trace;

pub fn inspect(path: &Path, limit: usize, kind: Option<&str>, no_verify: bool) -> Result<ExitCode> {
    let trace = crate::load_trace_with(path, !no_verify)
        .with_context(|| format!("{} is not a valid trace", path.display()))?;
    let h = &trace.header;
    println!("trace    {}", path.display());
    println!("test     {}", h.test);
    println!(
        "sources  {}{}",
        h.sources,
        h.seed.map(|s| format!(" (seed {s})")).unwrap_or_default()
    );
    println!("written  {} (format v{})", h.created_by, h.version);
    println!(
        "outcome  {}{}",
        trace.outcome,
        trace
            .message
            .as_deref()
            .map(|m| format!(": {m}"))
            .unwrap_or_default()
    );
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &trace.events {
        *counts.entry(e.kind.as_str()).or_default() += 1;
    }
    let summary: Vec<String> = counts.iter().map(|(k, n)| format!("{k}={n}")).collect();
    println!("events   {} [{}]", trace.events.len(), summary.join(" "));
    println!("decisions {}", trace.decisions().len());
    let selected: Vec<_> = trace
        .events
        .iter()
        .filter(|e| kind.is_none_or(|k| e.kind.as_str() == k))
        .collect();
    let shown = if limit == 0 {
        selected.len()
    } else {
        limit.min(selected.len())
    };
    for e in &selected[..shown] {
        println!("  {}", e.describe());
    }
    if shown < selected.len() {
        println!(
            "  … {} more (use --limit 0 to show all)",
            selected.len() - shown
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn diff(a: &Path, b: &Path, context: usize) -> Result<ExitCode> {
    let ta =
        crate::load_trace(a).with_context(|| format!("{} is not a valid trace", a.display()))?;
    let tb =
        crate::load_trace(b).with_context(|| format!("{} is not a valid trace", b.display()))?;
    let Some(i) = ta.first_difference(&tb) else {
        println!(
            "identical event streams ({} events; call sites are not compared)",
            ta.events.len()
        );
        return Ok(ExitCode::SUCCESS);
    };
    println!("first difference at event #{i}");
    for e in &ta.events[i.saturating_sub(context)..i] {
        println!("  = {}", e.describe());
    }
    let show = |label: &str, t: &Trace, p: &Path| match t.events.get(i) {
        Some(e) => println!("  {label} {}   ({})", e.describe(), p.display()),
        None => println!(
            "  {label} <end of trace after {} events>   ({})",
            t.events.len(),
            p.display()
        ),
    };
    show("-", &ta, a);
    show("+", &tb, b);
    Ok(ExitCode::from(1))
}

pub fn validate(paths: &[PathBuf]) -> Result<ExitCode> {
    let mut bad = 0;
    for p in paths {
        match crate::load_trace(p) {
            Ok(t) => println!(
                "ok       {} ({} events, {})",
                p.display(),
                t.events.len(),
                t.outcome
            ),
            Err(e) => {
                bad += 1;
                println!("INVALID  {}: {e}", p.display());
            }
        }
    }
    Ok(if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}
