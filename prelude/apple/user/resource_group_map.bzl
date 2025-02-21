# Copyright (c) Meta Platforms, Inc. and affiliates.
#
# This source code is licensed under both the MIT license found in the
# LICENSE-MIT file in the root directory of this source tree and the Apache
# License, Version 2.0 found in the LICENSE-APACHE file in the root directory
# of this source tree.

load(
    "@prelude//apple:resource_groups.bzl",
    "ResourceGraph",
    "ResourceGroupInfo",
    "create_resource_graph",
    "get_resource_graph_node_map_func",
)
load(
    "@prelude//cxx:groups.bzl",
    "compute_mappings",
    "create_group",
    "parse_groups_definitions",
)
load("@prelude//user:rule_spec.bzl", "RuleRegistrationSpec")
load("@prelude//decls/common.bzl", "Traversal")

def resource_group_map_attr():
    return attrs.option(attrs.dep(providers = [ResourceGroupInfo]), default = None)

def _impl(ctx: "context") -> ["provider"]:
    resource_groups = parse_groups_definitions(ctx.attrs.map)
    resource_groups_deps = [mapping.root.node for group in resource_groups for mapping in group.mappings]
    resource_graph = create_resource_graph(
        ctx = ctx,
        labels = [],
        deps = resource_groups_deps,
        exported_deps = [],
    )
    resource_graph_node_map = get_resource_graph_node_map_func(resource_graph)()
    mappings = compute_mappings(
        groups = [
            create_group(
                group = group,
                # User provided mappings may contain entries that don't support
                # ResourceGraph, which `create_resource_graph` removes above.
                # So make sure we remove them from the mappings too, otherwise
                # `compute_mappings` crashes on the inconsistency.
                mappings = [
                    mapping
                    for mapping in group.mappings
                    if mapping.root == None or ResourceGraph in mapping.root.node
                ],
            )
            for group in resource_groups
        ],
        graph_map = resource_graph_node_map,
    )
    return [
        DefaultInfo(),
        ResourceGroupInfo(groups = resource_groups, groups_hash = hash(str(resource_groups)), mappings = mappings),
    ]

registration_spec = RuleRegistrationSpec(
    name = "resource_group_map",
    impl = _impl,
    attrs = {
        "map": attrs.list(attrs.tuple(attrs.string(), attrs.list(attrs.tuple(attrs.dep(), attrs.enum(Traversal), attrs.option(attrs.string()))))),
    },
)
