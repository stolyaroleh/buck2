/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

use std::str::FromStr;
use std::sync::Arc;

use allocative::Allocative;
use buck2_build_api::actions::RegisteredAction;
use buck2_build_api::build_signals::NodeDuration;
use buck2_events::span::SpanId;
use dupe::Dupe;
use smallvec::SmallVec;

use crate::BuildInfo;
use crate::NodeKey;

pub(crate) trait BuildListenerBackend {
    fn process_node(
        &mut self,
        key: NodeKey,
        value: Option<Arc<RegisteredAction>>,
        duration: NodeDuration,
        dep_keys: impl Iterator<Item = NodeKey>,
        span_ids: SmallVec<[SpanId; 1]>,
    );

    fn process_top_level_target(
        &mut self,
        analysis: NodeKey,
        artifacts: impl Iterator<Item = NodeKey>,
    );

    fn finish(self) -> anyhow::Result<BuildInfo>;

    fn name() -> CriticalPathBackendName;
}

#[derive(Copy, Clone, Dupe, derive_more::Display, Allocative)]
pub enum CriticalPathBackendName {
    #[display(fmt = "longest-path-graph")]
    LongestPathGraph,
    #[display(fmt = "default")]
    Default,
}

impl FromStr for CriticalPathBackendName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "longest-path-graph" {
            return Ok(Self::LongestPathGraph);
        }

        if s == "default" {
            return Ok(Self::Default);
        }

        Err(anyhow::anyhow!("Invalid backend name: `{}`", s))
    }
}
