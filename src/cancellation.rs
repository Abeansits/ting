//! Scoped cancellation context; each forum and its worker threads share a token.
use anyhow::Result;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub type Token = Arc<AtomicBool>;
thread_local! { static CURRENT: RefCell<Option<Token>> = const { RefCell::new(None) }; }

#[derive(Debug)]
pub struct Interrupted;
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Execution interrupted; any completed forum checkpoints remain available for resume"
        )
    }
}
impl std::error::Error for Interrupted {}

pub struct Scope {
    previous: Option<Token>,
    _not_send: PhantomData<Rc<()>>,
}
impl Scope {
    fn enter(token: Option<Token>) -> Self {
        Self {
            previous: CURRENT.with(|slot| slot.replace(token)),
            _not_send: PhantomData,
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

pub fn current() -> Option<Token> {
    CURRENT.with(|slot| slot.borrow().clone())
}
pub fn requested() -> bool {
    current().is_some_and(|token| token.load(Ordering::SeqCst))
}
pub fn check() -> Result<()> {
    if requested() {
        Err(Interrupted.into())
    } else {
        Ok(())
    }
}
pub fn is_interrupted(error: &anyhow::Error) -> bool {
    error.downcast_ref::<Interrupted>().is_some()
}
pub fn with_token<T>(token: Option<Token>, run: impl FnOnce() -> T) -> T {
    let _scope = Scope::enter(token);
    run()
}

/// First signal requests cleanup; a second signal forces process exit.
pub fn install_for_cli() -> Result<Scope> {
    let token = Arc::new(AtomicBool::new(false));
    let signal_token = token.clone();
    ctrlc::set_handler(move || {
        if signal_token.swap(true, Ordering::SeqCst) {
            std::process::exit(130);
        }
    })?;
    Ok(Scope::enter(Some(token)))
}
