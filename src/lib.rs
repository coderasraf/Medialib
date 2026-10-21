//! medialib: index media files in local folders and HTTP directory listings,
//! search the index locally, and play results in a player or the browser.
//!
//! Two front ends share this library: the `medialib` CLI (src/main.rs) and
//! the `medialib-desktop` app (src/desktop.rs).

pub mod assistant;
pub mod config;
pub mod crawler;
pub mod index;
pub mod jobs;
pub mod library;
pub mod naming;
pub mod player;
pub mod tmdb;
pub mod web;
