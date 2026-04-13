//! Nous Language Server Protocol implementation.
//!
//! Provides real-time diagnostics, hover info, and goto definition
//! for .ns files in VS Code and other editors.

use std::collections::HashMap;

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::*;
use lsp_types::Uri as Url;

/// Run the LSP server on stdio.
pub fn run_lsp() {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
            DiagnosticOptions {
                identifier: Some("nous".to_string()),
                inter_file_dependencies: false,
                workspace_diagnostics: false,
                work_done_progress_options: WorkDoneProgressOptions::default(),
            },
        )),
        ..Default::default()
    })
    .unwrap();

    let init_params = match connection.initialize(server_capabilities) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("LSP init error: {e}");
            return;
        }
    };

    let mut docs: HashMap<Url, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req).unwrap_or(false) {
                    break;
                }
                handle_request(&connection, &docs, req);
            }
            Message::Notification(notif) => {
                handle_notification(&connection, &mut docs, notif);
            }
            Message::Response(_) => {}
        }
    }

    io_threads.join().ok();
}

fn handle_request(conn: &Connection, docs: &HashMap<Url, String>, req: Request) {
    match req.method.as_str() {
        "textDocument/hover" => {
            let params: HoverParams =
                serde_json::from_value(req.params).unwrap_or_else(|_| {
                    panic!("invalid hover params");
                });
            let uri = &params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;

            let hover_text = if let Some(source) = docs.get(uri) {
                get_hover_info(source, pos)
            } else {
                None
            };

            let result = hover_text.map(|text| {
                Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: text,
                    }),
                    range: None,
                }
            });

            let resp = Response::new_ok(req.id, result);
            conn.sender.send(Message::Response(resp)).ok();
        }
        _ => {
            let resp = Response::new_err(req.id, -32601, "method not found".into());
            conn.sender.send(Message::Response(resp)).ok();
        }
    }
}

fn handle_notification(conn: &Connection, docs: &mut HashMap<Url, String>, notif: Notification) {
    match notif.method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenTextDocumentParams =
                serde_json::from_value(notif.params).unwrap();
            let uri = params.text_document.uri.clone();
            docs.insert(uri.clone(), params.text_document.text.clone());
            publish_diagnostics(conn, &uri, &params.text_document.text);
        }
        "textDocument/didChange" => {
            let params: DidChangeTextDocumentParams =
                serde_json::from_value(notif.params).unwrap();
            let uri = params.text_document.uri.clone();
            if let Some(change) = params.content_changes.into_iter().last() {
                docs.insert(uri.clone(), change.text.clone());
                publish_diagnostics(conn, &uri, &change.text);
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseTextDocumentParams =
                serde_json::from_value(notif.params).unwrap();
            docs.remove(&params.text_document.uri);
        }
        _ => {}
    }
}

/// Parse + type check the document and publish diagnostics.
fn publish_diagnostics(conn: &Connection, uri: &Url, source: &str) {
    let mut diagnostics = Vec::new();

    // Phase 1: Parse
    match nous_parser::parse(source) {
        Err(e) => {
            diagnostics.push(Diagnostic {
                range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some("nous/parser".into()),
                message: e.to_string(),
                ..Default::default()
            });
        }
        Ok(program) => {
            // Phase 2: Type check
            let mut checker = nous_types::TypeChecker::new();
            if let Err(errors) = checker.check(&program) {
                for err in &errors {
                    diagnostics.push(Diagnostic {
                        range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                        severity: Some(DiagnosticSeverity::ERROR),
                        source: Some("nous/types".into()),
                        message: err.to_string(),
                        ..Default::default()
                    });
                }
            }

            // Phase 3: Verify contracts (Z3)
            let verifier = nous_verify::Verifier::new();
            match verifier.verify(&program) {
                Ok(result) => {
                    for diag in &result.diagnostics {
                        let severity = match diag.level {
                            nous_verify::diagnostic::DiagLevel::Error => DiagnosticSeverity::ERROR,
                            nous_verify::diagnostic::DiagLevel::Warning => DiagnosticSeverity::WARNING,
                            nous_verify::diagnostic::DiagLevel::Info => DiagnosticSeverity::INFORMATION,
                        };
                        let line = if diag.location.span.line > 0 {
                            diag.location.span.line as u32 - 1
                        } else {
                            0
                        };
                        let col = if diag.location.span.col > 0 {
                            diag.location.span.col as u32 - 1
                        } else {
                            0
                        };
                        let mut msg = format!("[{}] {}", diag.code, diag.constraint);
                        if let Some(ce) = &diag.counterexample {
                            let ce_str: Vec<String> = ce.iter().map(|(k,v)| format!("{k}={v}")).collect();
                            msg.push_str(&format!(" (counterexample: {})", ce_str.join(", ")));
                        }
                        if !diag.fix_strategies.is_empty() {
                            msg.push_str(&format!(" | fix: {}", diag.fix_strategies[0].description));
                        }
                        diagnostics.push(Diagnostic {
                            range: Range::new(
                                Position::new(line, col),
                                Position::new(line, col + 10),
                            ),
                            severity: Some(severity),
                            source: Some("nous/verify".into()),
                            message: msg,
                            ..Default::default()
                        });
                    }
                    // Synthesized function info
                    for w in &result.warnings {
                        if w.contains("synthesized") {
                            diagnostics.push(Diagnostic {
                                range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                                severity: Some(DiagnosticSeverity::INFORMATION),
                                source: Some("nous/synthesis".into()),
                                message: w.clone(),
                                ..Default::default()
                            });
                        }
                    }
                }
                Err(errors) => {
                    for err in &errors {
                        diagnostics.push(Diagnostic {
                            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
                            severity: Some(DiagnosticSeverity::ERROR),
                            source: Some("nous/verify".into()),
                            message: err.to_string(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }

    let notif = Notification::new(
        "textDocument/publishDiagnostics".into(),
        PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics,
            version: None,
        },
    );
    conn.sender.send(Message::Notification(notif)).ok();
}

/// Get hover information at a position.
fn get_hover_info(source: &str, pos: Position) -> Option<String> {
    let line_num = pos.line as usize;
    let col = pos.character as usize;

    let line = source.lines().nth(line_num)?;

    // Find the word at the cursor position
    let start = line[..col].rfind(|c: char| !c.is_alphanumeric() && c != '_').map_or(0, |i| i + 1);
    let end = line[col..].find(|c: char| !c.is_alphanumeric() && c != '_').map_or(line.len(), |i| col + i);
    let word = &line[start..end];

    if word.is_empty() {
        return None;
    }

    // Provide hover info based on the word
    match word {
        // Keywords
        "fn" => Some("**fn** — Function declaration\n\n```nous\nfn name(params) -> ReturnType\n  require precondition\n  ensure postcondition\n  body\n```".into()),
        "flow" => Some("**flow** — Multi-step operation with automatic rollback\n\n```nous\nflow name(params) -> Result[T, E]\n  step s1 = ...\n    rollback: ...\n  step s2 = ...\n    rollback: ...\n```\nIf any step fails, completed steps roll back in reverse order.".into()),
        "entity" => Some("**entity** — Immutable record type with named fields\n\n```nous\nentity Account\n  id : Text\n  balance : Int\n  invariant balance >= 0\n```".into()),
        "state" => Some("**state** — State machine type with verified transitions\n\n```nous\nstate Order\n  Pending -[confirm]-> Confirmed\n  Confirmed -[ship]-> Shipped\n```\nCompiler verifies reachability and liveness.".into()),
        "require" => Some("**require** — Precondition (caller's obligation)\n\nZ3 verifies at compile time. Enforced at runtime.\nViolation produces a counterexample.".into()),
        "ensure" => Some("**ensure** — Postcondition (implementation's guarantee)\n\nIf the function has no body, the compiler **synthesizes** the implementation from ensure constraints.\n\n```nous\nfn add(a: Int, b: Int) -> Int\n  ensure result == a + b\n-- no body needed\n```".into()),
        "effect" => Some("**effect** — Side effect declaration\n\nFunctions must declare all effects. Calling an effectful function from a pure context is a compile error.\n\n```nous\neffect Db.write, Http.request\n```".into()),
        // Types
        "Int" => Some("**Int** — Integer type (64-bit signed)".into()),
        "Nat" => Some("**Nat** — Natural number (unsigned, >= 0)".into()),
        "Bool" => Some("**Bool** — Boolean (true / false)".into()),
        "Text" => Some("**Text** — UTF-8 string".into()),
        "Result" => Some("**Result[T, E]** — Success or failure\n\n`Ok(value)` or `Err(error)`. Must be consumed (linear type).".into()),
        "Void" => Some("**Void** — No meaningful value".into()),
        _ => {
            // Check if it's on a require/ensure line
            let trimmed = line.trim();
            if trimmed.starts_with("require") || trimmed.starts_with("ensure") {
                Some(format!("**Constraint**: `{}`\n\nVerified by Z3 SMT solver at compile time.", trimmed))
            } else if trimmed.starts_with("effect") {
                Some(format!("**Effect declaration**: `{}`\n\nAll callees' effects must be a subset of this declaration.", trimmed))
            } else {
                None
            }
        }
    }
}
