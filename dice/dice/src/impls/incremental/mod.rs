/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

//!
//! The incrementality module of BUCK
//!
//! This is responsible for performing incremental caching and invalidations
//! with multiple versions in-flight at the same time.
//!

use std::any::Any;
use std::borrow::Cow;
use std::fmt::Debug;

use allocative::Allocative;
use dupe::Dupe;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt;
use more_futures::cancellation::future::TerminationStatus;
use tokio::sync::oneshot;
use tracing::Instrument;

use crate::api::activation_tracker::ActivationData;
use crate::api::error::DiceResult;
use crate::arc::Arc;
use crate::impls::core::graph::history::CellHistory;
use crate::impls::core::graph::types::VersionedGraphKey;
use crate::impls::core::graph::types::VersionedGraphResult;
use crate::impls::core::state::CoreStateHandle;
use crate::impls::core::state::StateRequest;
use crate::impls::core::versions::VersionEpoch;
use crate::impls::evaluator::AsyncEvaluator;
use crate::impls::evaluator::SyncEvaluator;
use crate::impls::events::DiceEventDispatcher;
use crate::impls::key::DiceKey;
use crate::impls::key::ParentKey;
use crate::impls::key_index::DiceKeyIndex;
use crate::impls::task::dice::DiceTask;
use crate::impls::task::handle::DiceTaskHandle;
use crate::impls::task::promise::DicePromise;
use crate::impls::task::spawn_dice_task;
use crate::impls::task::PreviouslyCancelledTask;
use crate::impls::user_cycle::UserCycleDetectorData;
use crate::impls::value::DiceComputedValue;
use crate::versions::VersionNumber;
use crate::versions::VersionRanges;
use crate::ActivationTracker;

#[cfg(test)]
mod tests;

/// The incremental engine that manages all the handling of the results of a
/// specific key, performing the recomputation if necessary
///
/// The computation of an identical request (same key and version) is
/// automatically deduplicated, so that identical requests share the same set of
/// work. It is guaranteed that there is at most one computation in flight at a
/// time if they share the same key and version.
#[derive(Allocative)]
pub(crate) struct IncrementalEngine {
    state: CoreStateHandle,
    version_epoch: VersionEpoch,
}

impl Debug for IncrementalEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncrementalEngine").finish_non_exhaustive()
    }
}

impl IncrementalEngine {
    fn new(state: CoreStateHandle, version_epoch: VersionEpoch) -> Self {
        Self {
            state,
            version_epoch,
        }
    }

    pub(crate) fn spawn_for_key(
        k: DiceKey,
        version_epoch: VersionEpoch,
        eval: AsyncEvaluator,
        cycles: UserCycleDetectorData,
        events_dispatcher: DiceEventDispatcher,
        previously_cancelled_task: Option<PreviouslyCancelledTask>,
    ) -> DiceTask {
        let eval_dupe = eval.dupe();
        spawn_dice_task(&*eval.user_data.spawner, &eval.user_data, move |handle| {
            async move {
                if let Some(previous) = previously_cancelled_task {
                    debug!(msg = "waiting for previously cancelled task");
                    match previous.termination.await {
                        TerminationStatus::Finished => {
                            // old task actually finished, so just use that result
                            handle.finished(
                                previous
                                    .previous
                                    .get_finished_value()
                                    .expect("A finished task must have been completed"),
                            );

                            debug!(msg = "previously cancelled task actually finished");

                            return Box::new(()) as Box<dyn Any + Send + 'static>;
                        }
                        _ => {
                            // continue re-evaluating
                        }
                    }
                }

                let engine =
                    IncrementalEngine::new(eval_dupe.dice.state_handle.dupe(), version_epoch);

                engine
                    .eval_entry_versioned(k, eval_dupe, cycles, events_dispatcher, handle)
                    .await;

                Box::new(()) as Box<dyn Any + Send + 'static>
            }
            .instrument(debug_span!(parent: None, "spawned_dice_task", k = ?k, v = %eval.per_live_version_ctx.get_version(), v_epoch = %version_epoch))
            .boxed()
        })
    }

    #[instrument(
        level = "debug",
        skip(state, promise, eval, event_dispatcher),
        fields(k = ?k, version = %v),
    )]
    pub(crate) fn project_for_key(
        state: CoreStateHandle,
        promise: DicePromise,
        k: DiceKey,
        v: VersionNumber,
        version_epoch: VersionEpoch,
        eval: SyncEvaluator,
        event_dispatcher: DiceEventDispatcher,
    ) -> DiceResult<DiceComputedValue> {
        promise.get_or_complete(|| {
            event_dispatcher.started(k);

            debug!(msg = "running projection");

            let eval_result = eval.evaluate(k);

            debug!(msg = "projection finished. updating caches");

            let res = match eval_result {
                Ok(res) => {
                    // send the update but don't wait for it
                    match res.value.dupe().into_valid_value() {
                        Ok(value) => {
                            let (tx, _rx) = tokio::sync::oneshot::channel();
                            state.request(StateRequest::UpdateComputed {
                                key: VersionedGraphKey::new(v, k),
                                epoch: version_epoch,
                                storage: res.storage,
                                value,
                                deps: Arc::new(res.deps.into_iter().collect()),
                                resp: tx,
                            });
                            // TODO(bobyf) consider if we want to block and wait for the cache
                        }
                        Err(_) => {}
                    }

                    Ok(res.value)
                }
                Err(e) => Err(e),
            };

            debug!(msg = "update future completed");
            event_dispatcher.finished(k);

            res.map(|val| DiceComputedValue::new(val, Arc::new(CellHistory::verified(v))))
        })
    }

    async fn eval_entry_versioned(
        &self,
        k: DiceKey,
        eval: AsyncEvaluator,
        mut cycles: UserCycleDetectorData,
        events_dispatcher: DiceEventDispatcher,
        task_handle: DiceTaskHandle<'_>,
    ) {
        let v = eval.per_live_version_ctx.get_version();
        let (tx, rx) = oneshot::channel();
        self.state.request(StateRequest::LookupKey {
            key: VersionedGraphKey::new(v, k),
            resp: tx,
        });

        let state_result = rx.await.unwrap();

        match state_result {
            VersionedGraphResult::Match(entry) => {
                debug!(
                    msg = "found existing entry with matching version in cache. reusing result.",
                );
                task_handle.finished(Ok(entry))
            }
            VersionedGraphResult::Compute => {
                cycles.start_computing_key(
                    k,
                    &eval.dice.key_index,
                    eval.user_data.cycle_detector.as_deref(),
                );
                self.compute(k, eval, cycles, &events_dispatcher, task_handle)
                    .await;
            }

            VersionedGraphResult::CheckDeps(mismatch) => {
                cycles.start_computing_key(
                    k,
                    &eval.dice.key_index,
                    eval.user_data.cycle_detector.as_deref(),
                );
                task_handle.checking_deps();

                let deps_changed = {
                    events_dispatcher.check_deps_started(k);
                    scopeguard::defer! {
                        events_dispatcher.check_deps_finished(k);
                    }

                    self.compute_whether_dependencies_changed(
                        ParentKey::Some(k), // the computing of deps is triggered by this key as the parent
                        eval.dupe(),
                        &mismatch.verified_versions,
                        mismatch.deps_to_validate,
                        &cycles,
                    )
                    .await
                };

                match deps_changed {
                    DidDepsChange::Changed | DidDepsChange::NoDeps => {
                        self.compute(k, eval, cycles, &events_dispatcher, task_handle)
                            .await;
                    }
                    DidDepsChange::NoChange(deps) => {
                        cycles.finished_computing_key(
                            &eval.dice.key_index,
                            eval.user_data.cycle_detector.as_deref(),
                        );

                        debug!(
                            msg = "reusing previous value because deps didn't change. Updating caches"
                        );

                        report_key_activation(
                            &eval.dice.key_index,
                            eval.user_data.activation_tracker.as_deref(),
                            k,
                            deps.iter().copied(),
                            ActivationData::Reused,
                        );

                        // report reuse
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        self.state.request(StateRequest::UpdateComputed {
                            key: VersionedGraphKey::new(v, k),
                            epoch: self.version_epoch,
                            storage: eval.storage_type(k),
                            value: mismatch.entry,
                            deps,
                            resp: tx,
                        });

                        debug!(msg = "Update caches complete");

                        task_handle.finished(rx.await.unwrap())
                    }
                }
            }
        }
    }

    async fn compute(
        &self,
        k: DiceKey,
        eval: AsyncEvaluator,
        cycles: UserCycleDetectorData,
        event_dispatcher: &DiceEventDispatcher,
        task_handle: DiceTaskHandle<'_>,
    ) {
        task_handle.computing();

        event_dispatcher.started(k);
        scopeguard::defer! {
            event_dispatcher.finished(k);
        };

        let v = eval.per_live_version_ctx.get_version();

        // TODO(bobyf) these also make good locations where we want to perform instrumentation
        debug!(msg = "running evaluator");

        let eval_result = eval
            .evaluate(k, cycles, task_handle.cancellation_ctx())
            .await;

        let _guard = match task_handle.cancellation_ctx().try_to_disable_cancellation() {
            Some(g) => g,
            None => {
                debug!("evaluation cancelled, skipping cache updates");
                return;
            }
        };

        debug!(msg = "evaluation finished. updating caches");

        match eval_result {
            Ok(res) => {
                report_key_activation(
                    &eval.dice.key_index,
                    eval.user_data.activation_tracker.as_deref(),
                    k,
                    res.deps.iter().copied(),
                    res.evaluation_data.into_activation_data(),
                );

                match res.value.into_valid_value() {
                    Ok(value) => {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        self.state.request(StateRequest::UpdateComputed {
                            key: VersionedGraphKey::new(v, k),
                            epoch: self.version_epoch,
                            storage: res.storage,
                            value,
                            deps: Arc::new(res.deps.into_iter().collect()),
                            resp: tx,
                        });

                        task_handle.finished(rx.await.unwrap())
                    }
                    Err(value) => {
                        task_handle.finished(Ok(DiceComputedValue::new(
                            value,
                            Arc::new(CellHistory::verified(v)),
                        )));
                    }
                }
            }
            Err(e) => task_handle.finished(Err(e)),
        }

        debug!(msg = "update future completed");
    }

    /// determines if the given 'Dependency' has changed between versions 'last_version' and
    /// 'target_version'
    #[instrument(
        level = "debug",
        skip(self, eval, cycles),
        fields(version = %eval.per_live_version_ctx.get_version(), verified_versions = %verified_versions)
    )]
    async fn compute_whether_dependencies_changed(
        &self,
        parent_key: ParentKey,
        eval: AsyncEvaluator,
        verified_versions: &VersionRanges,
        deps: Arc<Vec<DiceKey>>,
        cycles: &UserCycleDetectorData,
    ) -> DidDepsChange {
        if deps.is_empty() {
            return DidDepsChange::NoDeps;
        }

        let mut fs: FuturesUnordered<_> = deps
            .iter()
            .map(|dep| {
                eval.per_live_version_ctx
                    .compute_opaque(
                        dep.dupe(),
                        parent_key,
                        &eval,
                        cycles.subrequest(*dep, &eval.dice.key_index),
                    )
                    .map(|r| r.map(|v| v.history().get_verified_ranges()))
            })
            .collect();

        let mut verified_versions = Cow::Borrowed(verified_versions);

        while let Some(dep_res) = fs.next().await {
            match dep_res {
                Ok(dep_version_ranges) => {
                    verified_versions =
                        Cow::Owned(verified_versions.intersect(&dep_version_ranges));
                    if verified_versions.is_empty() {
                        debug!(msg = "deps changed");
                        return DidDepsChange::Changed;
                    }
                }
                Err(_dice_err) => {
                    // we don't cache DiceErrors, so this must be because the dependency changed
                    // If the cycle/DiceError is real, we'll hit and propagate it when we recompute
                    // the parent key.
                    return DidDepsChange::Changed;
                }
            }
        }

        debug!(msg = "deps did not change");

        DidDepsChange::NoChange(deps)
    }
}

enum DidDepsChange {
    Changed,
    /// These deps did not change
    NoChange(Arc<Vec<DiceKey>>),
    NoDeps,
}

fn report_key_activation(
    key_index: &DiceKeyIndex,
    activation_tracker: Option<&dyn ActivationTracker>,
    key: DiceKey,
    deps: impl Iterator<Item = DiceKey>,
    activation_data: ActivationData,
) {
    if let Some(activation_tracker) = &activation_tracker {
        let key = key_index.get(key).as_any();
        let mut iter = deps.map(|dep| key_index.get(dep).as_any());
        activation_tracker.key_activated(key, &mut iter, activation_data);
    }
}

#[cfg(test)]
pub(crate) mod testing {

    use crate::impls::incremental::DidDepsChange;

    pub(crate) trait DidDepsChangeExt {
        fn is_changed(&self) -> bool;
    }

    impl DidDepsChangeExt for DidDepsChange {
        fn is_changed(&self) -> bool {
            match self {
                DidDepsChange::Changed => true,
                DidDepsChange::NoChange(..) => false,
                DidDepsChange::NoDeps => false,
            }
        }
    }
}
