//! A client for the Jobs mailbox, and nothing else.
//!
//! The mailbox (paulgsc/some-ui `apps/mailbox`) is a Cloudflare Worker over a
//! D1 database where the cloud routine and home leave each other files: the
//! routine puts the day's digest in `brief`, home puts the phone's decisions
//! and scouts in `decisions` and `scout`, and the reflections routine puts
//! its word sets and dramas in `scene` and `drama`. Each side holds its own bearer
//! token, and the Worker decides what each token may read and put. This crate
//! speaks its `/v1` API: [`Mailbox::list`], [`Mailbox::get`], [`Mailbox::put`],
//! and, with the archive token, [`Mailbox::archive`], [`Mailbox::wanted`] and
//! [`Mailbox::restore`].
//! It does not know what a digest is, which subject a file is for, or when to
//! ask; `file_host`'s `mailbox` module decides all three.
//!
//! ## The seam
//!
//! Putting a request on the wire is a [`MailboxTransport`], a generic
//! parameter as `push_kit`'s transport is: one per binary, known when it is
//! compiled. [`ReqwestTransport`] is the one shipped, behind the
//! `reqwest-transport` feature; a test brings its own.
//!
//! ## What never leaves this crate
//!
//! **The token.** It is held in a [`Token`], whose `Debug` prints
//! `[redacted]` and which has no `Display`, so neither a [`Mailbox`] nor a
//! [`Request`] can print it. The transport reads it once, to set the header.
//! **Bodies.** A [`MailboxError`] carries a status, a kind, or the mailbox's
//! own short refusal, never a file's bytes, and [`ReqwestTransport`] strips
//! the URL from its errors.

mod client;
pub mod transport;

pub use client::{Archive, Folder, ItemMeta, Mailbox, MailboxError, Put, Restore, Wanted, PAGE_LIMIT};
pub use transport::{MailboxTransport, Method, Request, Response, Token};

#[cfg(feature = "reqwest-transport")]
pub use transport::reqwest_transport::ReqwestTransport;
