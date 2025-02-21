/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

use allocative::Allocative;
use buck2_interpreter::types::target_label::StarlarkTargetLabel;
use buck2_node::attrs::inspect_options::AttrInspectOptions;
use buck2_node::nodes::unconfigured::TargetNode;
use derive_more::Display;
use dupe::Dupe;
use starlark::any::ProvidesStaticType;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::starlark_type;
use starlark::values::structs::AllocStruct;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::StarlarkDocs;

use crate::bxl::starlark_defs::file_set::StarlarkFileNode;
use crate::bxl::starlark_defs::nodes::unconfigured::attribute::StarlarkCoercedAttr;
use crate::bxl::starlark_defs::nodes::unconfigured::attribute::StarlarkTargetNodeCoercedAttributes;

pub mod attribute;

#[derive(Debug, Display, ProvidesStaticType, Allocative, StarlarkDocs)]
#[derive(NoSerialize)] // TODO probably should be serializable the same as how queries serialize
#[display(fmt = "{:?}", self)]
#[starlark_docs(directory = "bxl")]
pub struct StarlarkTargetNode(pub TargetNode);

starlark_simple_value!(StarlarkTargetNode);

impl<'v> StarlarkValue<'v> for StarlarkTargetNode {
    starlark_type!("unconfigured_target_node");

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new();
        RES.methods(target_node_value_methods)
    }
}

impl<'a> UnpackValue<'a> for StarlarkTargetNode {
    fn expected() -> String {
        "target node".to_owned()
    }

    fn unpack_value(value: starlark::values::Value<'a>) -> Option<Self> {
        value
            .downcast_ref::<Self>()
            .map(|value| Self(value.0.dupe()))
    }
}

/// Methods for unconfigured target node.
#[starlark_module]
fn target_node_value_methods(builder: &mut MethodsBuilder) {
    /// DO NOT USE. Will be deprecated.
    ///
    /// Gets the coerced attributes from the unconfigured target node. Returns an iterable `starlark_attributes`
    /// object.
    ///
    /// Sample usage:
    /// ```text
    /// def _impl_attributes(ctx):
    ///     target_node = ctx.uquery().eval("owner('path/to/file')")[0]
    ///     for attr in target_node.attributes:
    ///         ctx.output.print(attr)
    /// ```
    #[starlark(attribute)]
    fn attributes<'v>(this: StarlarkTargetNode, heap: &Heap) -> anyhow::Result<Value<'v>> {
        Ok(heap.alloc(StarlarkTargetNodeCoercedAttributes {
            inner: heap.alloc(this),
        }))
    }

    /// Gets the coerced attributes from the unconfigured target node. Returns a struct.
    ///
    /// Sample usage:
    /// ```text
    /// def _impl_attributes(ctx):
    ///     target_node = ctx.uquery().eval("owner('path/to/file')")[0]
    ///     ctx.output.print(target_node.attrs.my_attr)
    /// ```
    #[starlark(attribute)]
    fn attrs<'v>(this: StarlarkTargetNode, heap: &Heap) -> anyhow::Result<Value<'v>> {
        let attrs_iter = this.0.attrs(AttrInspectOptions::All);
        let attrs = attrs_iter.map(|a| {
            (
                a.name,
                StarlarkCoercedAttr(a.value.clone(), this.0.label().pkg().dupe()),
            )
        });

        Ok(heap.alloc(AllocStruct(attrs)))
    }

    /// Gets the label from the unconfigured target node.
    ///
    /// Sample usage:
    /// ```text
    /// def _impl_label(ctx):
    ///     target_node = ctx.uquery().eval("owner('path/to/file')")[0]
    ///     ctx.output.print(target_node.label)
    /// ```
    #[starlark(attribute)]
    fn label(this: &StarlarkTargetNode) -> anyhow::Result<StarlarkTargetLabel> {
        Ok(this.0.label().dupe().into())
    }

    /// Gets the buildfile path from the unconfigured target node.
    ///
    /// Sample usage:
    /// ```text
    /// def _impl_label(ctx):
    ///     target_node = ctx.uquery().eval("owner('path/to/file')")[0]
    ///     ctx.output.print(target_node.buildfile_path)
    /// ```
    #[starlark(attribute)]
    fn buildfile_path(this: &StarlarkTargetNode) -> anyhow::Result<StarlarkFileNode> {
        Ok(StarlarkFileNode(this.0.buildfile_path().path()))
    }
}
