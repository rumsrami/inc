//! Core lang transformations that understand Scheme language idiosyncrasies
//!
//! Home for renaming, lifting, type checks and everything else.
use crate::{
    compiler::state::State,
    core::{
        Code,
        Expr::{self, *},
    },
};

use std::collections::HashMap;

/// Rename/mangle all references to unique names
pub fn rename(prog: &[Expr]) -> Vec<Expr> {
    prog.iter().map(|e| mangle(&HashMap::<String, i64>::new(), e)).collect()
}

/// Lift all expressions in a program.
// Some values must be lifted to the top level to ease certain stages of the
// compiler. Actions are specific to the types - strings and symbols are added
// to a lookup table and lambda definitions are raised to top level.
pub fn lift(s: &mut State, prog: &[Expr]) -> Vec<Expr> {
    prog.iter().map({ |expr| lift1(s, &expr) }).collect()
}

/// Mangle a single expression with letrec support.
///
// A sub expression in let binding is evaluated with the complete environment
// including the one being defined only if the subexpresison captures the
// closure with another let or lambda, otherwise evaluate with only the rest of
// the bindings.
fn mangle(env: &HashMap<String, i64>, prog: &Expr) -> Expr {
    match prog {
        Identifier(i) => match env.get(i.as_str()) {
            Some(n) => Identifier(format!("{}.{}", i, n)),
            None => Identifier(i.to_string()),
        },

        Let { bindings, body } => {
            // Collect all the names about to be bound for evaluating body
            let mut all = env.clone();
            for (name, _) in bindings {
                all.entry(name.into()).and_modify(|e| *e += 1).or_insert(0);
            }

            let bindings = bindings.iter().map(|(name, value)| {
                // Collect all the names excluding the one being defined now
                let mut rest = env.clone();
                for (n, _) in bindings {
                    if n != name {
                        rest.entry(n.into()).and_modify(|e| *e += 1).or_insert(0);
                    }
                }

                let value = match value {
                    Let { .. } => mangle(&all, value),
                    Lambda(_) => mangle(&all, value),
                    _ => mangle(&rest, value),
                };

                let index = all.get(name).unwrap();
                let name = format!("{}.{}", name, index);

                (name, value)
            });

            Let {
                bindings: bindings.collect(),
                body: body.iter().map(|b| mangle(&all, b)).collect(),
            }
        }

        List(list) => List(list.iter().map(|l| mangle(env, l)).collect()),

        Cond { pred, then, alt } => Cond {
            pred: box mangle(env, pred),
            then: box mangle(env, then),
            alt: alt.as_ref().map(|u| box mangle(env, u)),
        },

        Lambda(Code { name, formals, free, body }) => Lambda(Code {
            name: name.clone(),
            formals: formals.clone(),
            free: free.clone(),
            body: body.iter().map(|b| mangle(env, b)).collect(),
        }),

        // All literals and constants evaluate to itself
        v => v.clone(),
    }
}

fn lift1(s: &mut State, prog: &Expr) -> Expr {
    match prog {
        Str(reference) => {
            if !s.strings.contains_key(reference) {
                s.strings.insert(reference.clone(), s.strings.len());
            }
            Str(reference.clone())
        }

        Symbol(reference) => {
            if !s.symbols.contains_key(reference) {
                s.symbols.insert(reference.clone(), s.symbols.len());
            }
            Symbol(reference.clone())
        }

        Let { bindings, body } => {
            // Rest is all the name bindings that are not functions
            let mut rest: Vec<(String, Expr)> = vec![];

            for (name, expr) in bindings {
                match expr {
                    Lambda(Code { formals, free, body, .. }) => {
                        let code = Code {
                            name: Some(name.to_string()),
                            formals: formals.clone(),
                            free: free.clone(),
                            body: lift(s, body),
                        };
                        s.functions.insert(name.to_string(), code);
                    }

                    _ => rest.push((name.clone(), lift1(s, expr))),
                };
            }

            let body = body.iter().map({ |b| lift1(s, b) }).collect();

            Let { bindings: rest, body }
        }

        List(list) => List(list.iter().map({ |l| lift1(s, l) }).collect()),

        Cond { pred, then, alt } => Cond {
            pred: box lift1(s, pred),
            then: box lift1(s, then),
            alt: alt.as_ref().map({ |box e| box lift1(s, &e) }),
        },

        // A literal lambda must be in an inline calling position
        Lambda(Code { .. }) => unimplemented!("inline λ"),

        e => e.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{Error, Expr},
        parser,
        parser::parse,
    };
    use pretty_assertions::assert_eq;

    fn one(x: Result<Vec<Expr>, Error>) -> Expr {
        x.unwrap()[0].clone()
    }

    #[test]
    fn shadow1() {
        let x = one(parse("(let ((x 1)) (let ((x 2)) (+ x x)))"));
        let y = one(parse("(let ((x.0 1)) (let ((x.1 2)) (+ x.1 x.1)))"));

        assert_eq!(y, mangle(&HashMap::<String, i64>::new(), &x));
    }

    #[test]
    fn shadow2() {
        let x = one(parse("(let ((t (cons 1 2))) (let ((t t)) (let ((t t)) (let ((t t)) t))))"));
        let y = one(parse(
            "(let ((t.0 (cons 1 2))) (let ((t.1 t.0)) (let ((t.2 t.1)) (let ((t.3 t.2)) t.3))))",
        ));

        assert_eq!(y, mangle(&HashMap::<String, i64>::new(), &x));
    }

    #[test]
    fn shadow3() {
        let x = one(parse(
            "(let ((x ()))
               (let ((x (cons x x)))
                 (let ((x (cons x x)))
                   (let ((x (cons x x)))
                     (cons x x)))))",
        ));

        let y = one(parse(
            "(let ((x.0 ()))
               (let ((x.1 (cons x.0 x.0)))
                 (let ((x.2 (cons x.1 x.1)))
                   (let ((x.3 (cons x.2 x.2)))
                     (cons x.3 x.3)))))",
        ));

        assert_eq!(y, mangle(&HashMap::<String, i64>::new(), &x));
    }

    #[test]
    fn alias() {
        let x = one(parse("(let ((x 1)) (let ((x x)) (+ x x)))"));
        let y = one(parse("(let ((x.0 1)) (let ((x.1 x.0)) (+ x.1 x.1)))"));

        assert_eq!(y, mangle(&HashMap::<String, i64>::new(), &x));
    }

    #[test]
    fn letrec() {
        let x = one(parse(
            "(let ((f (lambda (x) (g x x)))
                   (g (lambda (x y) (+ x y))))
               (f 12))",
        ));

        let y = one(parse(
            "(let ((f.0 (lambda (x) (g.0 x x)))
                   (g.0 (lambda (x y) (+ x y))))
               (f.0 12))",
        ));

        assert_eq!(y, mangle(&HashMap::<String, i64>::new(), &x));
    }

    #[test]
    fn recursive() {
        let x = one(parse(
            "(let ((f (lambda (x)
                        (if (zero? x)
                          1
                          (* x (f (dec x))))))) (f 5))",
        ));

        let y = one(parse(
            "(let ((f.0 (lambda (x)
                          (if (zero? x)
                            1
                            (* x (f.0 (dec x))))))) (f.0 5))",
        ));

        assert_eq!(y, mangle(&HashMap::<String, i64>::new(), &x));
    }

    #[test]
    fn lift_simple() {
        let prog = r"(let ((id (lambda (x) x))) (id 42))";
        let mut s: State = Default::default();

        let expr = match parser::parse(prog) {
            Ok(r) => r,
            Err(e) => panic!(e),
        };

        let e = lift(&mut s, &expr);

        assert_eq!(
            s.functions.get("id").unwrap(),
            &Code {
                name: Some("id".into()),
                formals: vec!["x".into()],
                free: vec![],
                body: vec!["x".into()],
            }
        );

        assert_eq!(e[0], Let { bindings: vec![], body: vec![List(vec!["id".into(), Number(42)])] });
    }

    #[test]
    fn lift_recursive() {
        let prog = r"(let ((e (lambda (x) (if (zero? x) #t (o (dec x)))))
                           (o (lambda (x) (if (zero? x) #f (e (dec x))))))
                       (e 25)))";

        let mut s: State = Default::default();

        let expr = match parser::parse(prog) {
            Ok(r) => r,
            Err(e) => panic!(e),
        };

        let e = lift(&mut s, &expr);

        assert_eq!(
            s.functions.get("e").unwrap(),
            &Code {
                name: Some("e".into()),
                formals: vec!["x".into()],
                free: vec![],
                body: vec![Cond {
                    pred: box List(vec!["zero?".into(), "x".into()]),
                    then: box Boolean(true),
                    alt: Some(box List(vec!["o".into(), List(vec!["dec".into(), "x".into()])]))
                }]
            }
        );

        assert_eq!(
            s.functions.get("o").unwrap(),
            &Code {
                name: Some("o".into()),
                formals: vec!["x".into()],
                free: vec![],
                body: vec![Cond {
                    pred: box List(vec!["zero?".into(), "x".into()]),
                    then: box Boolean(false),
                    alt: Some(box List(vec!["e".into(), List(vec!["dec".into(), "x".into()])]))
                }]
            }
        );

        assert_eq!(e[0], Let { bindings: vec![], body: vec![List(vec!["e".into(), Number(25)])] });
    }
}