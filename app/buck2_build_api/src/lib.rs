/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

#![feature(async_closure)]
#![feature(box_patterns)]
#![feature(iter_order_by)]
#![feature(try_blocks)]
#![feature(try_trait_v2)]
#![feature(never_type)]
#![feature(provide_any)]
// Plugins
#![cfg_attr(feature = "gazebo_lint", feature(plugin))]
#![cfg_attr(feature = "gazebo_lint", allow(deprecated))] // :(
#![cfg_attr(feature = "gazebo_lint", plugin(gazebo_lint))]

#[macro_use]
extern crate starlark;

pub mod actions;
pub mod analysis;
pub mod artifact_groups;
pub mod attrs;
pub mod audit_dep_files;
pub mod audit_output;
pub mod build;
pub mod build_signals;
pub mod bxl;
pub mod calculation;
pub mod configuration;
pub mod configure_dice;
pub mod configure_targets;
pub mod context;
pub mod deferred;
pub mod dynamic;
pub mod interpreter;
pub mod keep_going;
pub mod nodes;
pub mod query;
pub mod spawner;
