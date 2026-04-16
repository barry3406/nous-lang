use pest::iterators::{Pair, Pairs};

use nous_ast::decl::*;
use nous_ast::decl::{TrustLevel, EnsureClause, Obligation};
use nous_ast::expr::*;
use nous_ast::program::Program;
use nous_ast::span::{Span, Spanned};
use nous_ast::types::*;

use crate::error::ParseError;
use crate::Rule;

// ── Helpers ─���────────────────────────────────────────

fn mk_span(pair: &Pair<'_, Rule>) -> Span {
    let s = pair.as_span();
    let (line, col) = s.start_pos().line_col();
    Span::new(s.start(), s.end(), line, col)
}

fn sp<T>(node: T, pair: &Pair<'_, Rule>) -> Spanned<T> {
    Spanned::new(node, mk_span(pair))
}

fn skip(pair: &Pair<'_, Rule>) -> bool {
    matches!(pair.as_rule(), Rule::BLOCK_START | Rule::BLOCK_END)
}

// ── Program ──────────────────────────────────────────

pub fn build_program(pairs: Pairs<'_, Rule>) -> Result<Program, ParseError> {
    let mut decls = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                if inner.as_rule() == Rule::declaration {
                    let child = inner.into_inner().next().unwrap();
                    let span = mk_span(&child);
                    decls.push(Spanned::new(build_decl(child)?, span));
                }
            }
        }
    }
    Ok(Program { declarations: decls })
}

// ── Declarations ─────────────────────────────────────

fn build_decl(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    match p.as_rule() {
        Rule::namespace_decl => {
            let path = dotted(p.into_inner().next().unwrap());
            Ok(Decl::Namespace(NamespaceDecl { path }))
        }
        Rule::use_decl => {
            let raw = p.as_str().to_string();
            let mut inner = p.into_inner();
            let path = dotted(inner.next().unwrap());
            let wildcard = raw.contains(".*");
            Ok(Decl::Use(UseDecl { path, wildcard }))
        }
        Rule::type_decl => {
            let mut inner = p.into_inner();
            let name = ident_str(&mut inner);
            let tp = next_rule(&mut inner, Rule::type_expr);
            let span = mk_span(&tp);
            Ok(Decl::Type(TypeDecl { name, ty: Spanned::new(build_type(tp)?, span) }))
        }
        Rule::entity_decl => build_entity(p),
        Rule::enum_decl => build_enum(p),
        Rule::state_decl => build_state(p),
        Rule::effect_decl => {
            let dp = p.into_inner().next().unwrap();
            Ok(Decl::Effect(EffectDecl { name: dotted(dp).join(".") }))
        }
        Rule::capability_decl => {
            let mut inner = content(p);
            let dp = inner.next().unwrap();
            let name = dotted(dp).join(".");
            let mut params = Vec::new();
            let mut return_type = Spanned::dummy(TypeExpr::Void);
            let mut cap = CapabilityDecl {
                name: name.clone(), params: vec![], return_type: Spanned::dummy(TypeExpr::Void),
                idempotent_by: None, timeout: None, compensate: None,
                retry: None, confirm_by: None, trust: TrustLevel::Observed,
            };
            for child in inner {
                match child.as_rule() {
                    Rule::param_list => params = build_params(child)?,
                    Rule::type_expr => {
                        let span = mk_span(&child);
                        return_type = Spanned::new(build_type(child)?, span);
                    }
                    Rule::capability_attr => {
                        let attr = child.into_inner().next().unwrap();
                        match attr.as_rule() {
                            Rule::cap_idempotent => {
                                cap.idempotent_by = Some(attr.into_inner().next().unwrap().as_str().to_string());
                            }
                            Rule::cap_timeout => {
                                cap.timeout = Some(attr.into_inner().next().unwrap().as_str().parse().unwrap_or(30));
                            }
                            Rule::cap_compensate => {
                                let ep = attr.into_inner().next().unwrap();
                                let span = mk_span(&ep);
                                cap.compensate = Some(Spanned::new(build_expr(ep)?, span));
                            }
                            Rule::cap_retry => {
                                cap.retry = Some(attr.into_inner().next().unwrap().as_str().parse().unwrap_or(3));
                            }
                            Rule::cap_confirm => {
                                cap.confirm_by = Some(attr.into_inner().next().unwrap().as_str().to_string());
                            }
                            Rule::cap_trust => {
                                let tl = attr.into_inner().next().unwrap();
                                cap.trust = parse_trust_level(tl.as_str());
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            cap.params = params;
            cap.return_type = return_type;
            Ok(Decl::Capability(cap))
        }
        Rule::fn_decl => build_fn(p),
        Rule::flow_decl => build_flow(p),
        Rule::endpoint_decl => build_endpoint(p),
        Rule::handler_decl => build_handler(p),
        Rule::main_decl => build_main(p),
        _ => Err(err("declaration", &p)),
    }
}

// ── Entity ───────────────────────────────────────────

fn build_entity(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let name = ident_str(&mut inner);
    let mut fields = Vec::new();
    let mut invariants = Vec::new();
    for m in inner {
        if m.as_rule() == Rule::entity_member {
            let child = m.into_inner().next().unwrap();
            match child.as_rule() {
                Rule::field_decl => {
                    let span = mk_span(&child);
                    fields.push(Spanned::new(build_field(child)?, span));
                }
                Rule::invariant_clause => {
                    let ep = child.into_inner().next().unwrap();
                    let span = mk_span(&ep);
                    invariants.push(Spanned::new(build_expr(ep)?, span));
                }
                _ => {}
            }
        }
    }
    Ok(Decl::Entity(EntityDecl { name, fields, invariants }))
}

fn build_field(p: Pair<'_, Rule>) -> Result<Field, ParseError> {
    let mut inner = p.into_inner();
    let name = ident_str(&mut inner);
    let tp = inner.next().unwrap();
    let span = mk_span(&tp);
    Ok(Field { name, ty: Spanned::new(build_type(tp)?, span) })
}

// ── Enum ─────────────────────────────────────────────

fn build_enum(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let name = ident_str(&mut inner);
    let variants = inner
        .filter(|c| c.as_rule() == Rule::enum_variant)
        .map(|v| {
            let mut vi = v.into_inner();
            let vname = ident_str(&mut vi);
            let fields = vi
                .filter(|f| f.as_rule() == Rule::field_decl)
                .map(build_field)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(EnumVariant { name: vname, fields })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok(Decl::Enum(EnumDecl { name, variants }))
}

// ── State machine ────────────────────────────────────

fn build_state(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let name = ident_str(&mut inner);
    // terminal_state entries are informational — verifier discovers them from transition graph
    let transitions = inner
        .filter(|c| c.as_rule() == Rule::transition)
        .map(|t| {
            let mut ti = t.into_inner();
            let from = ident_str(&mut ti);
            let action = ident_str(&mut ti);
            let mut params = Vec::new();
            let mut to = String::new();
            for part in ti {
                match part.as_rule() {
                    Rule::transition_params => {
                        for pp in part.into_inner() {
                            if pp.as_rule() == Rule::param {
                                params.push(build_param(pp)?);
                            }
                        }
                    }
                    Rule::ident => to = part.as_str().to_string(),
                    _ => {}
                }
            }
            Ok(Transition { from, action, params, to })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok(Decl::State(StateDecl { name, transitions }))
}

// ── Function ─────────────────────────────────────────

fn build_fn(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let name = ident_str(&mut inner);

    let mut params = Vec::new();
    let mut return_type = Spanned::dummy(TypeExpr::Void);
    let mut contract = Contract { requires: vec![], ensures: vec![], effects: vec![], trust: TrustLevel::default(), obligations: vec![] };
    let mut stmts: Vec<Spanned<Expr>> = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::param_list => params = build_params(child)?,
            Rule::type_expr => {
                let span = mk_span(&child);
                return_type = Spanned::new(build_type(child)?, span);
            }
            Rule::contract_clause => collect_contract(&mut contract, child)?,
            Rule::stmt => {
                let span = mk_span(&child);
                stmts.push(Spanned::new(build_stmt(child)?, span));
            }
            _ => {}
        }
    }

    let body = if stmts.len() == 1 {
        stmts.remove(0)
    } else {
        Spanned::dummy(Expr::Block(stmts))
    };

    Ok(Decl::Fn(FnDecl { name, params, return_type, contract, body }))
}

fn build_params(p: Pair<'_, Rule>) -> Result<Vec<Param>, ParseError> {
    p.into_inner()
        .filter(|c| c.as_rule() == Rule::param)
        .map(build_param)
        .collect()
}

fn build_param(p: Pair<'_, Rule>) -> Result<Param, ParseError> {
    let mut inner = p.into_inner();
    let name = ident_str(&mut inner);
    let tp = inner.next().unwrap();
    let span = mk_span(&tp);
    Ok(Param { name, ty: Spanned::new(build_type(tp)?, span) })
}

fn collect_contract(c: &mut Contract, p: Pair<'_, Rule>) -> Result<(), ParseError> {
    let child = p.into_inner().next().unwrap();
    match child.as_rule() {
        Rule::require_clause => {
            let mut trust = TrustLevel::Checked;
            let mut exprs: Vec<_> = Vec::new();
            for sub in child.into_inner() {
                match sub.as_rule() {
                    Rule::trust_level => trust = parse_trust_level(sub.as_str()),
                    Rule::expr => exprs.push(sub),
                    _ => {}
                }
            }
            let cond = exprs.remove(0);
            let cond_span = mk_span(&cond);
            let else_expr = if !exprs.is_empty() {
                let e = exprs.remove(0);
                let span = mk_span(&e);
                Some(Spanned::new(build_expr(e)?, span))
            } else { None };
            c.requires.push(RequireClause {
                condition: Spanned::new(build_expr(cond)?, cond_span),
                else_expr,
                trust,
            });
        }
        Rule::ensure_clause => {
            let mut trust = TrustLevel::Checked;
            let mut expr_pair = None;
            for sub in child.into_inner() {
                match sub.as_rule() {
                    Rule::trust_level => trust = parse_trust_level(sub.as_str()),
                    Rule::expr => expr_pair = Some(sub),
                    _ => {}
                }
            }
            let ep = expr_pair.unwrap();
            let span = mk_span(&ep);
            c.ensures.push(EnsureClause {
                condition: Spanned::new(build_expr(ep)?, span),
                trust,
            });
        }
        Rule::trust_clause => {
            let tl = child.into_inner().next().unwrap();
            c.trust = parse_trust_level(tl.as_str());
        }
        Rule::obligation_clause => {
            let mut inner = child.into_inner();
            let name = ident_str(&mut inner);
            let desc = inner.next().map(|s| {
                let raw = s.as_str();
                raw[1..raw.len()-1].to_string()
            });
            c.obligations.push(Obligation { name, description: desc });
        }
        Rule::effect_clause => {
            for dp in child.into_inner() {
                if dp.as_rule() == Rule::dotted_path {
                    c.effects.push(dotted(dp).join("."));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ── Flow ─────────────────────────────────────────────

fn build_flow(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let name = ident_str(&mut inner);

    let mut params = Vec::new();
    let mut return_type = Spanned::dummy(TypeExpr::Void);
    let mut contract = Contract { requires: vec![], ensures: vec![], effects: vec![], trust: TrustLevel::default(), obligations: vec![] };
    let mut steps = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::param_list => params = build_params(child)?,
            Rule::type_expr => {
                let span = mk_span(&child);
                return_type = Spanned::new(build_type(child)?, span);
            }
            Rule::contract_clause => collect_contract(&mut contract, child)?,
            Rule::flow_step => {
                let mut si = content(child);
                let step_name = ident_str(&mut si);
                let mut body_stmts = Vec::new();
                let mut rollback = Spanned::dummy(Expr::Void);
                for part in si {
                    match part.as_rule() {
                        Rule::stmt => {
                            let span = mk_span(&part);
                            body_stmts.push(Spanned::new(build_stmt(part)?, span));
                        }
                        Rule::expr => {
                            let span = mk_span(&part);
                            rollback = Spanned::new(build_expr(part)?, span);
                        }
                        _ => {}
                    }
                }
                let body = if body_stmts.len() == 1 {
                    body_stmts.remove(0)
                } else {
                    Spanned::dummy(Expr::Block(body_stmts))
                };
                steps.push(FlowStep { name: step_name, body, rollback });
            }
            _ => {}
        }
    }

    Ok(Decl::Flow(FlowDecl { name, params, return_type, contract, steps }))
}

// ── Endpoint ─────────────────────────────────────────

fn build_endpoint(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);

    let method = match inner.next().unwrap().as_str() {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
        "PATCH" => HttpMethod::Patch,
        _ => HttpMethod::Get,
    };
    let path = inner.next().unwrap().as_str().to_string();

    let mut input_fields = Vec::new();
    let mut output_mappings = Vec::new();
    let mut handler = Spanned::dummy(Expr::Void);

    for child in inner {
        match child.as_rule() {
            Rule::field_decl => {
                let span = mk_span(&child);
                input_fields.push(Spanned::new(build_field(child)?, span));
            }
            Rule::output_mapping => {
                let mut oi = child.into_inner();
                let status: u16 = oi.next().unwrap().as_str().parse().unwrap_or(200);
                let tp = oi.next().unwrap();
                let span = mk_span(&tp);
                output_mappings.push(OutputMapping { status, ty: Spanned::new(build_type(tp)?, span) });
            }
            Rule::expr => {
                let span = mk_span(&child);
                handler = Spanned::new(build_expr(child)?, span);
            }
            _ => {}
        }
    }

    Ok(Decl::Endpoint(EndpointDecl { method, path, input_fields, output_mappings, handler }))
}

// ── Handler & Main ───────────────────────────────────

fn build_handler(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let name = ident_str(&mut inner);
    let bindings = inner
        .filter(|c| c.as_rule() == Rule::handler_binding)
        .map(|b| {
            let mut bi = b.into_inner();
            let effect = ident_str(&mut bi);
            let ep = bi.next().unwrap();
            let span = mk_span(&ep);
            Ok(HandlerBinding { effect, implementation: Spanned::new(build_expr(ep)?, span) })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;
    Ok(Decl::Handler(HandlerDecl { name, bindings }))
}

fn build_main(p: Pair<'_, Rule>) -> Result<Decl, ParseError> {
    let mut inner = content(p);
    let mut handlers = Vec::new();
    let mut stmts = Vec::new();

    for child in inner {
        match child.as_rule() {
            Rule::ident => handlers.push(child.as_str().to_string()),
            Rule::stmt => {
                let span = mk_span(&child);
                stmts.push(Spanned::new(build_stmt(child)?, span));
            }
            _ => {}
        }
    }

    let body = if stmts.len() == 1 {
        stmts.remove(0)
    } else {
        Spanned::dummy(Expr::Block(stmts))
    };
    Ok(Decl::Main(MainDecl { handlers, body }))
}

// ── Types ────────────────────────────────────────────

fn build_type(p: Pair<'_, Rule>) -> Result<TypeExpr, ParseError> {
    match p.as_rule() {
        Rule::type_expr => build_type(p.into_inner().next().unwrap()),
        Rule::type_union => {
            let parts: Vec<_> = p.into_inner().collect();
            if parts.len() == 1 {
                build_type(parts.into_iter().next().unwrap())
            } else {
                let vs = parts.into_iter().map(|c| {
                    let span = mk_span(&c);
                    Ok(Spanned::new(build_type(c)?, span))
                }).collect::<Result<Vec<_>, ParseError>>()?;
                Ok(TypeExpr::Union(vs))
            }
        }
        Rule::type_refined => {
            let mut inner = p.into_inner();
            let base_p = inner.next().unwrap();
            let base_span = mk_span(&base_p);
            let base = build_type(base_p)?;
            if let Some(constraint_p) = inner.find(|c| c.as_rule() == Rule::expr) {
                let cs = mk_span(&constraint_p);
                Ok(TypeExpr::Refined {
                    base: Box::new(Spanned::new(base, base_span)),
                    constraint: Box::new(Spanned::new(build_expr(constraint_p)?, cs)),
                })
            } else {
                Ok(base)
            }
        }
        Rule::type_primary => build_type(p.into_inner().next().unwrap()),
        Rule::named_type => Ok(TypeExpr::Named(p.into_inner().next().unwrap().as_str().to_string())),
        Rule::generic_type => {
            let mut inner = p.into_inner();
            let name = ident_str(&mut inner);
            let args = inner
                .filter(|c| c.as_rule() == Rule::type_expr)
                .map(|c| { let s = mk_span(&c); Ok(Spanned::new(build_type(c)?, s)) })
                .collect::<Result<Vec<_>, ParseError>>()?;
            Ok(TypeExpr::Generic { name, args })
        }
        Rule::tuple_type => {
            let ts = p.into_inner()
                .filter(|c| c.as_rule() == Rule::type_expr)
                .map(|c| { let s = mk_span(&c); Ok(Spanned::new(build_type(c)?, s)) })
                .collect::<Result<Vec<_>, ParseError>>()?;
            Ok(TypeExpr::Tuple(ts))
        }
        Rule::void_type => Ok(TypeExpr::Void),
        _ => Err(err("type", &p)),
    }
}

// ─�� Statements ───────────────────────────────────────

fn build_stmt(p: Pair<'_, Rule>) -> Result<Expr, ParseError> {
    let child = p.into_inner().next().unwrap();
    match child.as_rule() {
        Rule::let_stmt => {
            let mut inner = child.into_inner();
            let pat_p = inner.next().unwrap();
            let ps = mk_span(&pat_p);
            let pattern = Box::new(Spanned::new(build_pattern(pat_p)?, ps));
            let mut ty = None;
            let mut value = Spanned::dummy(Expr::Void);
            for part in inner {
                match part.as_rule() {
                    Rule::type_expr => {
                        let s = mk_span(&part);
                        ty = Some(Spanned::new(build_type(part)?, s));
                    }
                    Rule::expr => {
                        let s = mk_span(&part);
                        value = Spanned::new(build_expr(part)?, s);
                    }
                    _ => {}
                }
            }
            Ok(Expr::Let { pattern, ty, value: Box::new(value) })
        }
        Rule::require_stmt => {
            let mut parts = child.into_inner().filter(|c| c.as_rule() == Rule::expr);
            let cond = parts.next().unwrap();
            let cs = mk_span(&cond);
            let else_expr = parts.next().map(|e| {
                let s = mk_span(&e);
                build_expr(e).map(|ex| Box::new(Spanned::new(ex, s)))
            }).transpose()?;
            Ok(Expr::Require {
                condition: Box::new(Spanned::new(build_expr(cond)?, cs)),
                else_expr,
            })
        }
        Rule::return_stmt => {
            let ep = child.into_inner().next().unwrap();
            let s = mk_span(&ep);
            Ok(Expr::Return(Box::new(Spanned::new(build_expr(ep)?, s))))
        }
        Rule::expr_stmt => build_expr(child.into_inner().next().unwrap()),
        _ => Err(err("statement", &child)),
    }
}

// ── Expressions ──────────────────────────────────────

fn build_expr(p: Pair<'_, Rule>) -> Result<Expr, ParseError> {
    match p.as_rule() {
        Rule::expr => build_expr(p.into_inner().next().unwrap()),
        Rule::pipe_expr => {
            let mut inner = p.into_inner().peekable();
            let first = inner.next().unwrap();
            let mut expr = build_expr(first)?;
            while let Some(func_p) = inner.next() {
                if func_p.as_rule() != Rule::ident { continue; }
                let func_name = func_p.as_str().to_string();
                let mut args = Vec::new();
                if inner.peek().is_some_and(|n| n.as_rule() == Rule::arg_list) {
                    let al = inner.next().unwrap();
                    for a in al.into_inner() {
                        if a.as_rule() == Rule::expr {
                            let s = mk_span(&a);
                            args.push(Spanned::new(build_expr(a)?, s));
                        }
                    }
                }
                expr = Expr::Pipe {
                    value: Box::new(Spanned::dummy(expr)),
                    func: Box::new(Spanned::dummy(Expr::Ident(func_name))),
                    args,
                };
            }
            Ok(expr)
        }
        Rule::or_expr => build_binop_chain(p, BinOp::Or),
        Rule::and_expr => build_binop_chain(p, BinOp::And),
        Rule::compare_expr => {
            let mut inner: Vec<_> = p.into_inner().collect();
            if inner.len() == 1 { return build_expr(inner.remove(0)); }
            let left_p = inner.remove(0);
            let ls = mk_span(&left_p);
            let op_p = inner.remove(0);
            let right_p = inner.remove(0);
            let rs = mk_span(&right_p);
            let op = match op_p.as_str() {
                "==" => BinOp::Eq, "/=" => BinOp::Neq,
                "<=" => BinOp::Lte, ">=" => BinOp::Gte,
                "<" => BinOp::Lt, ">" => BinOp::Gt,
                _ => BinOp::Eq,
            };
            Ok(Expr::BinOp {
                op,
                left: Box::new(Spanned::new(build_expr(left_p)?, ls)),
                right: Box::new(Spanned::new(build_expr(right_p)?, rs)),
            })
        }
        Rule::implies_expr => {
            let mut inner: Vec<_> = p.into_inner().collect();
            if inner.len() == 1 { return build_expr(inner.remove(0)); }
            let left_p = inner.remove(0); let ls = mk_span(&left_p);
            let right_p = inner.remove(0); let rs = mk_span(&right_p);
            Ok(Expr::BinOp {
                op: BinOp::Implies,
                left: Box::new(Spanned::new(build_expr(left_p)?, ls)),
                right: Box::new(Spanned::new(build_expr(right_p)?, rs)),
            })
        }
        Rule::add_expr => build_arith_chain(p, |s| match s { "+" => BinOp::Add, _ => BinOp::Sub }),
        Rule::mul_expr => build_arith_chain(p, |s| match s { "*" => BinOp::Mul, "/" => BinOp::Div, _ => BinOp::Mod }),
        Rule::unary_expr => build_expr(p.into_inner().next().unwrap()),
        Rule::neg_expr => {
            let inner = p.into_inner().next().unwrap();
            let s = mk_span(&inner);
            Ok(Expr::UnaryOp { op: UnaryOp::Neg, operand: Box::new(Spanned::new(build_expr(inner)?, s)) })
        }
        Rule::not_expr => {
            let inner = p.into_inner().next().unwrap();
            let s = mk_span(&inner);
            Ok(Expr::UnaryOp { op: UnaryOp::Not, operand: Box::new(Spanned::new(build_expr(inner)?, s)) })
        }
        Rule::postfix_expr => {
            let mut inner = p.into_inner();
            let prim = inner.next().unwrap();
            let span = mk_span(&prim);
            let mut expr = build_expr(prim)?;
            let mut cur_span = span;
            for op in inner {
                let actual = if op.as_rule() == Rule::postfix_op {
                    op.into_inner().next().unwrap()
                } else { op };
                let actual_rule = actual.as_rule();
                let actual_span = mk_span(&actual);
                match actual_rule {
                    Rule::field_access => {
                        let field = actual.into_inner().next().unwrap().as_str().to_string();
                        expr = Expr::FieldAccess { object: Box::new(Spanned::new(expr, cur_span)), field };
                    }
                    Rule::call_args => {
                        let args = actual.into_inner()
                            .filter(|c| c.as_rule() == Rule::arg_list)
                            .flat_map(|al| al.into_inner())
                            .filter(|c| c.as_rule() == Rule::expr)
                            .map(|a| { let s = mk_span(&a); build_expr(a).map(|e| Spanned::new(e, s)) })
                            .collect::<Result<Vec<_>, _>>()?;
                        expr = Expr::Call { func: Box::new(Spanned::new(expr, cur_span)), args };
                    }
                    Rule::try_op => {
                        expr = Expr::Try(Box::new(Spanned::new(expr, cur_span)));
                    }
                    Rule::prime_op => {
                        expr = Expr::Primed(Box::new(Spanned::new(expr, cur_span)));
                    }
                    _ => {}
                }
                cur_span = actual_span;
            }
            Ok(expr)
        }
        Rule::primary_expr => build_expr(p.into_inner().next().unwrap()),
        // Literals
        Rule::integer_lit => Ok(Expr::IntLit(p.as_str().parse().unwrap_or(0))),
        Rule::decimal_lit => Ok(Expr::DecLit(p.as_str().to_string())),
        Rule::string_lit => {
            let s = p.as_str();
            let inner = &s[1..s.len()-1];
            // Process escape sequences
            let mut result = String::with_capacity(inner.len());
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => result.push('\n'),
                        Some('t') => result.push('\t'),
                        Some('r') => result.push('\r'),
                        Some('\\') => result.push('\\'),
                        Some('"') => result.push('"'),
                        Some('\'') => result.push('\''),
                        Some(other) => { result.push('\\'); result.push(other); }
                        None => result.push('\\'),
                    }
                } else {
                    result.push(c);
                }
            }
            Ok(Expr::StringLit(result))
        }
        Rule::bool_lit => Ok(Expr::BoolLit(p.as_str() == "true")),
        Rule::void_lit | Rule::nothing_lit => Ok(Expr::Void),
        Rule::self_ref => Ok(Expr::SelfRef),
        Rule::ident_expr => Ok(Expr::Ident(p.into_inner().next().unwrap().as_str().to_string())),
        Rule::ident => Ok(Expr::Ident(p.as_str().to_string())),
        Rule::ok_expr => {
            let inner = p.into_inner().find(|c| c.as_rule() == Rule::expr).unwrap();
            let s = mk_span(&inner);
            Ok(Expr::Ok(Box::new(Spanned::new(build_expr(inner)?, s))))
        }
        Rule::err_expr => {
            let inner = p.into_inner().find(|c| c.as_rule() == Rule::expr).unwrap();
            let s = mk_span(&inner);
            Ok(Expr::Err(Box::new(Spanned::new(build_expr(inner)?, s))))
        }
        Rule::pre_expr => {
            let field = p.into_inner().next().unwrap().as_str().to_string();
            Ok(Expr::Pre(Box::new(Spanned::dummy(Expr::Ident(field)))))
        }
        Rule::list_lit => {
            let items = p.into_inner()
                .filter(|c| c.as_rule() == Rule::expr)
                .map(|c| { let s = mk_span(&c); build_expr(c).map(|e| Spanned::new(e, s)) })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::List(items))
        }
        Rule::tuple_or_paren => {
            let items: Vec<_> = p.into_inner().filter(|c| c.as_rule() == Rule::expr).collect();
            if items.len() == 1 {
                build_expr(items.into_iter().next().unwrap())
            } else {
                let parts = items.into_iter()
                    .map(|c| { let s = mk_span(&c); build_expr(c).map(|e| Spanned::new(e, s)) })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Expr::Tuple(parts))
            }
        }
        Rule::record_expr => {
            let mut inner = p.into_inner();
            let name = ident_str(&mut inner);
            let fields = inner
                .filter(|c| c.as_rule() == Rule::field_assign)
                .map(|fa| {
                    let mut fai = fa.into_inner();
                    let fname = ident_str(&mut fai);
                    let ep = fai.next().unwrap();
                    let s = mk_span(&ep);
                    build_expr(ep).map(|e| (fname, Spanned::new(e, s)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::Record { name, fields })
        }
        Rule::record_update_expr => {
            let mut inner = p.into_inner();
            let base_p = inner.find(|c| c.as_rule() == Rule::expr).unwrap();
            let bs = mk_span(&base_p);
            let base = build_expr(base_p)?;
            let updates = inner
                .filter(|c| c.as_rule() == Rule::field_assign)
                .map(|fa| {
                    let mut fai = fa.into_inner();
                    let fname = ident_str(&mut fai);
                    let ep = fai.next().unwrap();
                    let s = mk_span(&ep);
                    build_expr(ep).map(|e| (fname, Spanned::new(e, s)))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::RecordUpdate { base: Box::new(Spanned::new(base, bs)), updates })
        }
        Rule::if_expr => {
            let mut inner = p.into_inner();
            // First child is the condition (expr)
            let cond = inner.find(|c| c.as_rule() == Rule::expr).unwrap();
            let cs = mk_span(&cond);
            // Then if_branch for then, and optionally if_branch for else
            let mut branches: Vec<_> = inner.filter(|c| c.as_rule() == Rule::if_branch).collect();
            let then_branch_p = branches.remove(0);
            let ts = mk_span(&then_branch_p);
            let then_expr = build_if_branch(then_branch_p)?;
            let else_branch = if !branches.is_empty() {
                let else_p = branches.remove(0);
                let es = mk_span(&else_p);
                Some(Box::new(Spanned::new(build_if_branch(else_p)?, es)))
            } else { None };
            Ok(Expr::If {
                condition: Box::new(Spanned::new(build_expr(cond)?, cs)),
                then_branch: Box::new(Spanned::new(then_expr, ts)),
                else_branch,
            })
        }
        Rule::match_expr => {
            let mut inner = content(p);
            let scrutinee = inner.next().unwrap();
            let ss = mk_span(&scrutinee);
            let arms = inner
                .filter(|c| c.as_rule() == Rule::match_arm)
                .map(|arm| {
                    let mut ai = arm.into_inner();
                    let pat = ai.next().unwrap(); let ps = mk_span(&pat);
                    let body = ai.next().unwrap(); let bs = mk_span(&body);
                    Ok(MatchArm {
                        pattern: Spanned::new(build_pattern(pat)?, ps),
                        body: Spanned::new(build_expr(body)?, bs),
                    })
                })
                .collect::<Result<Vec<_>, ParseError>>()?;
            Ok(Expr::Match { scrutinee: Box::new(Spanned::new(build_expr(scrutinee)?, ss)), arms })
        }
        Rule::transaction_expr => {
            let stmts: Vec<_> = p.into_inner()
                .filter(|c| c.as_rule() == Rule::stmt)
                .map(|s| { let span = mk_span(&s); build_stmt(s).map(|e| Spanned::new(e, span)) })
                .collect::<Result<Vec<_>, _>>()?;
            let body = if stmts.len() == 1 { stmts.into_iter().next().unwrap() }
                       else { Spanned::dummy(Expr::Block(stmts)) };
            Ok(Expr::Transaction(Box::new(body)))
        }
        Rule::lambda_expr => {
            let mut inner = p.into_inner();
            let param_name = ident_str(&mut inner);
            let body_p = inner.next().unwrap();
            let bs = mk_span(&body_p);
            Ok(Expr::Lambda {
                params: vec![Param { name: param_name, ty: Spanned::dummy(TypeExpr::Named("_".into())) }],
                body: Box::new(Spanned::new(build_expr(body_p)?, bs)),
            })
        }
        _ => Err(err("expression", &p)),
    }
}

fn build_binop_chain(p: Pair<'_, Rule>, op: BinOp) -> Result<Expr, ParseError> {
    let mut inner: Vec<_> = p.into_inner().collect();
    if inner.len() == 1 { return build_expr(inner.remove(0)); }
    let mut iter = inner.into_iter();
    let first = iter.next().unwrap();
    let mut ls = mk_span(&first);
    let mut left = build_expr(first)?;
    for right_p in iter {
        let rs = mk_span(&right_p);
        let right = build_expr(right_p)?;
        left = Expr::BinOp { op: op.clone(), left: Box::new(Spanned::new(left, ls)), right: Box::new(Spanned::new(right, rs)) };
        ls = rs;
    }
    Ok(left)
}

fn build_arith_chain(p: Pair<'_, Rule>, op_fn: impl Fn(&str) -> BinOp) -> Result<Expr, ParseError> {
    let mut inner: Vec<_> = p.into_inner().collect();
    if inner.len() == 1 { return build_expr(inner.remove(0)); }
    let mut iter = inner.into_iter();
    let first = iter.next().unwrap();
    let mut ls = mk_span(&first);
    let mut left = build_expr(first)?;
    while let Some(op_p) = iter.next() {
        let op = op_fn(op_p.as_str());
        let right_p = iter.next().unwrap();
        let rs = mk_span(&right_p);
        let right = build_expr(right_p)?;
        left = Expr::BinOp { op, left: Box::new(Spanned::new(left, ls)), right: Box::new(Spanned::new(right, rs)) };
        ls = rs;
    }
    Ok(left)
}

// ── Patterns ─────────────────────────────────────────

fn build_pattern(p: Pair<'_, Rule>) -> Result<Pattern, ParseError> {
    match p.as_rule() {
        Rule::pattern => build_pattern(p.into_inner().next().unwrap()),
        Rule::wildcard_pat => Ok(Pattern::Wildcard),
        Rule::ident_pat => Ok(Pattern::Ident(p.into_inner().next().unwrap().as_str().to_string())),
        Rule::ok_pat => {
            let inner = p.into_inner().next().unwrap();
            let s = mk_span(&inner);
            Ok(Pattern::Constructor { name: "Ok".to_string(), fields: vec![Spanned::new(build_pattern(inner)?, s)] })
        }
        Rule::err_pat => {
            let inner = p.into_inner().next().unwrap();
            let s = mk_span(&inner);
            Ok(Pattern::Constructor { name: "Err".to_string(), fields: vec![Spanned::new(build_pattern(inner)?, s)] })
        }
        Rule::constructor_pat => {
            let mut inner = p.into_inner();
            let name = ident_str(&mut inner);
            let fields = inner
                .filter(|c| c.as_rule() == Rule::pattern)
                .map(|c| { let s = mk_span(&c); build_pattern(c).map(|pat| Spanned::new(pat, s)) })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Pattern::Constructor { name, fields })
        }
        Rule::tuple_pat => {
            let fields = p.into_inner()
                .filter(|c| c.as_rule() == Rule::pattern)
                .map(|c| { let s = mk_span(&c); build_pattern(c).map(|pat| Spanned::new(pat, s)) })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Pattern::Tuple(fields))
        }
        Rule::literal_pat => {
            let inner = p.into_inner().next().unwrap();
            Ok(Pattern::Literal(build_expr(inner)?))
        }
        _ => Ok(Pattern::Wildcard),
    }
}

/// Build an if-branch: either a block `BLOCK_START stmt* BLOCK_END` or a single expr.
fn build_if_branch(p: Pair<'_, Rule>) -> Result<Expr, ParseError> {
    let mut inner: Vec<_> = p.into_inner().filter(|c| !skip(c)).collect();
    if inner.is_empty() {
        return Ok(Expr::Void);
    }
    // If it's a single expr child, build it directly
    if inner.len() == 1 && inner[0].as_rule() == Rule::expr {
        return build_expr(inner.remove(0));
    }
    // Otherwise it's stmts from a block
    let mut stmts = Vec::new();
    for child in inner {
        match child.as_rule() {
            Rule::stmt => {
                let span = mk_span(&child);
                stmts.push(Spanned::new(build_stmt(child)?, span));
            }
            Rule::expr => {
                let span = mk_span(&child);
                stmts.push(Spanned::new(build_expr(child)?, span));
            }
            _ => {}
        }
    }
    if stmts.len() == 1 {
        Ok(stmts.remove(0).node)
    } else {
        Ok(Expr::Block(stmts))
    }
}

// ── Utilities ────────────────────────────────────────

fn parse_trust_level(s: &str) -> TrustLevel {
    match s {
        "proved" => TrustLevel::Proved,
        "checked" => TrustLevel::Checked,
        "observed" => TrustLevel::Observed,
        "assumed" => TrustLevel::Assumed,
        _ => TrustLevel::Checked,
    }
}

fn dotted(p: Pair<'_, Rule>) -> Vec<String> {
    p.into_inner().filter(|c| c.as_rule() == Rule::ident).map(|c| c.as_str().to_string()).collect()
}

fn ident_str<'a>(iter: &mut impl Iterator<Item = Pair<'a, Rule>>) -> String {
    iter.find(|c: &Pair<'a, Rule>| c.as_rule() == Rule::ident).unwrap().as_str().to_string()
}

fn next_rule<'a>(iter: &mut impl Iterator<Item = Pair<'a, Rule>>, rule: Rule) -> Pair<'a, Rule> {
    iter.find(|c: &Pair<'a, Rule>| c.as_rule() == rule).unwrap()
}

fn content(p: Pair<'_, Rule>) -> impl Iterator<Item = Pair<'_, Rule>> {
    p.into_inner().filter(|c| !skip(c))
}

fn err(expected: &str, pair: &Pair<'_, Rule>) -> ParseError {
    let (line, col) = pair.as_span().start_pos().line_col();
    ParseError::UnexpectedToken { line, col, expected: expected.into(), got: format!("{:?}", pair.as_rule()) }
}
