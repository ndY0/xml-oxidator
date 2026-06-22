/// Declarative macro for building a [`DescriptorTree`](crate::tree::descriptor::DescriptorTree).
///
/// # Syntax
///
/// ```text
/// build_tree!(
///     "root_tag" [streaming|capture] {
///         RuleName { field: value, ... },
///         "child_tag" [streaming|capture] {
///             AnotherRule { field: value },
///             "grandchild" { ... }
///         }
///     }
/// )
/// ```
///
/// - Each node starts with a **string literal** (the XML tag name).
/// - An optional **access mode** follows: `streaming` (default) or `capture`.
/// - The braced block contains a comma-separated mix of:
///   - **Rules** — struct-literal expressions (identified by starting with a path/ident).
///   - **Child nodes** — identified by starting with a string literal, recursively.
///
/// Returns `Result<DescriptorTree<Box<dyn Rule>>, BuilderError>`.
///
/// # Example
///
/// ```rust,ignore
/// use xml_oxydizer::build_tree;
///
/// let tree = build_tree!(
///     "catalog" streaming {
///         CheckVersion { expected: "3" },
///         "schema" capture {
///             ValidateFields { min_fields: 1 }
///         },
///         "entry" streaming {
///             CheckSku { pattern: "^[A-Z]+" },
///             "detail" {
///                 CheckText { max_len: 500 }
///             }
///         }
///     }
/// )?;
/// ```
#[macro_export]
macro_rules! build_tree {
    // Entry point: root node with explicit mode
    ($tag:literal $mode:ident { $($body:tt)* }) => {{
        let builder = $crate::tree::builder::TreeBuilder::<::std::boxed::Box<dyn $crate::rule::Rule>>::new($tag);
        let builder = $crate::build_tree!(@mode builder, $mode);
        let builder = $crate::build_tree!(@items builder, $($body)*);
        builder.build()
    }};

    // Entry point: root node without mode (defaults to streaming)
    ($tag:literal { $($body:tt)* }) => {{
        let builder = $crate::tree::builder::TreeBuilder::<::std::boxed::Box<dyn $crate::rule::Rule>>::new($tag);
        let builder = $crate::build_tree!(@items builder, $($body)*);
        builder.build()
    }};

    // ---- internal: apply access mode ----

    (@mode $builder:expr, streaming) => { $builder.streaming() };
    (@mode $builder:expr, capture) => { $builder.capture_subtree() };

    // ---- internal: parse body items ----

    // Base case: no more items
    (@items $builder:expr,) => { $builder };

    // Child node with explicit mode, then comma + rest
    (@items $builder:expr, $tag:literal $mode:ident { $($child_body:tt)* } , $($rest:tt)*) => {{
        let builder = $builder.node($tag);
        let builder = $crate::build_tree!(@mode builder, $mode);
        let builder = $crate::build_tree!(@items builder, $($child_body)*);
        let builder = builder.done();
        $crate::build_tree!(@items builder, $($rest)*)
    }};

    // Child node with explicit mode, last item
    (@items $builder:expr, $tag:literal $mode:ident { $($child_body:tt)* }) => {{
        let builder = $builder.node($tag);
        let builder = $crate::build_tree!(@mode builder, $mode);
        let builder = $crate::build_tree!(@items builder, $($child_body)*);
        builder.done()
    }};

    // Child node without mode, then comma + rest
    (@items $builder:expr, $tag:literal { $($child_body:tt)* } , $($rest:tt)*) => {{
        let builder = $builder.node($tag);
        let builder = $crate::build_tree!(@items builder, $($child_body)*);
        let builder = builder.done();
        $crate::build_tree!(@items builder, $($rest)*)
    }};

    // Child node without mode, last item
    (@items $builder:expr, $tag:literal { $($child_body:tt)* }) => {{
        let builder = $builder.node($tag);
        let builder = $crate::build_tree!(@items builder, $($child_body)*);
        builder.done()
    }};

    // Rule (struct literal via ident path), then comma + rest
    (@items $builder:expr, $($rule_path:ident)::+ { $($fields:tt)* } , $($rest:tt)*) => {{
        let builder = $builder.rule(::std::boxed::Box::new($($rule_path)::+ { $($fields)* }) as ::std::boxed::Box<dyn $crate::rule::Rule>);
        $crate::build_tree!(@items builder, $($rest)*)
    }};

    // Rule (struct literal via ident path), last item
    (@items $builder:expr, $($rule_path:ident)::+ { $($fields:tt)* }) => {{
        $builder.rule(::std::boxed::Box::new($($rule_path)::+ { $($fields)* }) as ::std::boxed::Box<dyn $crate::rule::Rule>)
    }};
}
