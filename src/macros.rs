/// The `Tools` enum [`tools!`] generates, with clap's `Subcommand` derived so
/// each tool is invocable from a shell.
#[cfg(feature = "cli")]
#[doc(hidden)]
#[macro_export]
macro_rules! __tools_cli_enum {
    ($($capitalized:tt),+) => {
        #[derive($crate::clap::Subcommand)]
        pub enum Tools {
            $(
                $capitalized(#[clap(flatten)] $capitalized),
            )+
        }
    };
}

#[cfg(not(feature = "cli"))]
#[doc(hidden)]
#[macro_export]
macro_rules! __tools_cli_enum {
    ($($capitalized:tt),+) => {
        compile_error!(
            "`tools!` derives clap's `Subcommand` for `Tools`, which needs mcplease's `cli` \
             feature; enable it, or write `tools!(cli = false, ...)`"
        );
    };
}

/// The `Tools` enum [`tools!`] generates without the clap derive.
#[doc(hidden)]
#[macro_export]
macro_rules! __tools_plain_enum {
    ($($capitalized:tt),+) => {
        pub enum Tools {
            $(
                $capitalized($capitalized),
            )+
        }
    };
}

/// Declare a server's tools: one module per tool, and the `Tools` enum that
/// dispatches to them.
///
/// ```ignore
/// mcplease::tools!(State, (Hello, hello, "hello"), (Goodbye, goodbye, "goodbye"));
/// ```
///
/// By default `Tools` derives clap's `Subcommand`, so each tool can be run
/// from a shell (see [`Cli`](crate::Cli)); that needs the `cli` feature and a
/// `clap::Args` derive on every tool. A server with no command line — one
/// behind another transport, or embedded — writes `tools!(cli = false, ...)`.
///
/// The choice is the caller's, not the feature's: cargo builds one mcplease
/// per build with the union of every dependent's features, so a macro that
/// followed the feature would change shape under one crate because another
/// crate in the same build asked for `cli`.
///
/// A crate that mounts the same tool types in more than one set cannot give
/// each set its own module per tool. `modules = false` (after `cli`, when both
/// are given) declares no modules and re-exports nothing: each tool is written
/// as `(Type, "name")`, and the types must already be in scope where the macro
/// is invoked.
///
/// ```ignore
/// use crate::shared::{Hello, LongGoodbye as Goodbye};
/// mcplease::tools!(cli = false, modules = false, State, (Hello, "hello"), (Goodbye, "goodbye"));
/// ```
#[macro_export]
macro_rules! tools {
    (cli = false, modules = false, $state:tt, $($tool:tt),+ $(,)?) => {
        $crate::__tools!(@in_scope __tools_plain_enum, $state, $($tool),+);
    };
    (cli = true, modules = false, $state:tt, $($tool:tt),+ $(,)?) => {
        $crate::__tools!(@in_scope __tools_cli_enum, $state, $($tool),+);
    };
    (modules = false, $state:tt, $($tool:tt),+ $(,)?) => {
        $crate::__tools!(@in_scope __tools_cli_enum, $state, $($tool),+);
    };
    (cli = false, $state:tt, $($tool:tt),+ $(,)?) => {
        $crate::__tools!(__tools_plain_enum, $state, $($tool),+);
    };
    (cli = true, $state:tt, $($tool:tt),+ $(,)?) => {
        $crate::__tools!(__tools_cli_enum, $state, $($tool),+);
    };
    ($state:tt, $($tool:tt),+ $(,)?) => {
        $crate::__tools!(__tools_cli_enum, $state, $($tool),+);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __tools {
    (@in_scope $enum:ident, $state:tt, $(($capitalized:tt, $string:literal)),+) => {
        $crate::__tools!(@enum $enum, $state, $(($capitalized, $string)),+);
    };
    ($enum:ident, $state:tt, $(($capitalized:tt, $lowercase:tt, $string:literal)),+) => {
        $(mod $lowercase;)+
        $(pub use $lowercase::$capitalized;)+

        $crate::__tools!(@enum $enum, $state, $(($capitalized, $string)),+);
    };
    (@enum $enum:ident, $state:tt, $(($capitalized:tt, $string:literal)),+) => {
        $crate::$enum!($($capitalized),+);

        impl std::fmt::Debug for Tools {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    $(Self::$capitalized(tool) => std::fmt::Debug::fmt(tool, f),)+
                }
            }
        }

        // Simple Deserialize implementation using serde_json::Value
        impl<'de> $crate::serde::Deserialize<'de> for Tools {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
            D: $crate::serde::Deserializer<'de>,
            {
                use $crate::serde::de;

                let value: $crate::serde_json::Value = $crate::serde::Deserialize::deserialize(deserializer)?;

                let obj = value.as_object()
                .ok_or_else(|| de::Error::custom("expected object"))?;

                let name = obj.get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| de::Error::missing_field("name"))?;

                // `params.arguments` is optional in the schema: a tool that
                // takes no arguments may be called without it, and absent
                // means the same as `{}`.
                let arguments = match obj.get("arguments") {
                    Some(arguments) => arguments.clone(),
                    None => $crate::serde_json::Value::Object($crate::serde_json::Map::new()),
                };

                match name {
                    $(
                        $string => $crate::serde_json::from_value(arguments)
                                       .map_err(de::Error::custom)
                                       .map(Tools::$capitalized),
                    )+
                    _ => Err(de::Error::unknown_variant(name, &[$($string),+])),
                }
            }
        }

        // Manual Serialize implementation to maintain the same format
        impl $crate::serde::Serialize for Tools {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
            S: $crate::serde::Serializer,
            {
                use $crate::serde::ser::SerializeStruct;

                let mut state = serializer.serialize_struct("Tools", 2)?;
                match self {
                    $(
                        Tools::$capitalized(args) => {
                            state.serialize_field("name", $string)?;
                            state.serialize_field("arguments", args)?;
                        }
                    )+
                }
                state.end()
            }
        }


        // `Tools` dispatches to tools rather than being one: each variant has
        // its own `Output` type, so it implements `Dispatch` rather than
        // `Tool`, erasing those types into a `CallToolResult`.
        impl $crate::traits::Dispatch<$state> for Tools {
            async fn call(
                self,
                state: $state,
                context: $crate::types::RequestContext,
            ) -> $crate::anyhow::Result<$crate::types::CallToolResult> {
                use $crate::traits::{Tool as _, ToolOutput as _};
                match self {
                    $(Tools::$capitalized(tool) => tool.execute(state, context).await.map(|output| {
                        $crate::types::CallToolResult::from_content(
                            output.to_content(),
                            output.structured_content(),
                        )
                    }),)+
                }
            }

            async fn call_to_text(
                self,
                state: $state,
                context: $crate::types::RequestContext,
            ) -> $crate::anyhow::Result<String> {
                use $crate::traits::{Tool as _, ToolOutput as _};
                match self {
                    $(Tools::$capitalized(tool) => {
                        tool.execute(state, context).await.map(|output| output.to_text())
                    })+
                }
            }
        }

        impl $crate::traits::AsToolsList for Tools {
            // Macro expansion order is the advertised order, which gives the
            // deterministic ordering the spec asks for: it lets clients cache
            // the list and keeps tool definitions stable in the model's
            // prompt cache. Do not sort or collect these through a HashMap.
            fn tools_list() -> Vec<$crate::types::Tool> {
                use $crate::traits::AsToolSchema;
                vec![
                    $(<$capitalized as AsToolSchema<$state>>::schema(),)+
                ]
            }
        }

        impl Tools {
            #[allow(dead_code)]
            pub fn name(&self) -> &str {
                match self {
                    $(Tools::$capitalized(_) => $string,)+
                }
            }
        }
    };
}

#[macro_export]
macro_rules! server_info {
    () => {
        $crate::types::Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    };
}
