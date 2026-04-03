use std::path::Path;

use heck::ToLowerCamelCase;
use specta::datatype::FunctionResultVariant;
use specta_typescript::{js_doc, ExportError, Typescript};

use crate::{ExportContext, LanguageExt};

use super::js_ts;

const GLOBALS: &str = include_str!("./globals.js");

impl LanguageExt for specta_jsdoc::JSDoc {
    type Error = ExportError;

    fn render(&self, cfg: &ExportContext) -> Result<String, Self::Error> {
        let dependant_types = cfg
            .type_map
            .into_iter()
            .map(|(_sid, ndt)| js_doc::typedef_named_datatype(&self.0, ndt, &cfg.type_map))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join("\n"))?;

        let tanstack_import = render_tanstack_import(cfg);

        let header = if tanstack_import.is_empty() {
            self.0.header.to_string()
        } else {
            format!("{}\n{tanstack_import}", self.0.header)
        };

        js_ts::render_all_parts::<Self>(
            cfg,
            &dependant_types,
            GLOBALS,
            &header,
            render_commands(&self.0, cfg)?,
            render_query_keys(cfg)?,
            render_queries(cfg)?,
            render_mutation_keys(cfg)?,
            render_mutations(cfg)?,
            render_events(&self.0, cfg)?,
            false,
        )
    }

    fn format(&self, path: &Path) -> Result<(), Self::Error> {
        if let Some(formatter) = self.0.formatter {
            formatter(path)?;
        }
        Ok(())
    }
}

pub fn render_tanstack_import(cfg: &ExportContext) -> String {
    if let Some(framework) = &cfg.tanstack {
        let has_queries = !cfg.queries.is_empty();
        let has_mutations = !cfg.mutations.is_empty();
        if has_queries || has_mutations {
            let mut imports = Vec::new();
            if has_queries {
                imports.push("queryOptions as TANSTACK_QUERY_OPTIONS");
            }
            if has_mutations {
                imports.push("mutationOptions as TANSTACK_MUTATION_OPTIONS");
            }
            return format!(
                "import {{ {} }} from \"{}\";\n",
                imports.join(", "),
                framework.package_name()
            );
        }
    }

    String::new()
}

fn render_commands(ts: &Typescript, cfg: &ExportContext) -> Result<String, ExportError> {
    let all_commands: Vec<_> = cfg
        .commands
        .iter()
        .chain(&cfg.queries)
        .chain(&cfg.mutations)
        .collect();

    let commands = all_commands
        .iter()
        .map(|function| {
            let jsdoc = {
                let ret_type =
                    js_ts::handle_result(function, &cfg.type_map, ts, cfg.error_handling)?;

                let mut builder = js_doc::Builder::default();

                if let Some(d) = function.deprecated() {
                    builder.push_deprecated(d);
                }

                if !function.docs().is_empty() {
                    builder.extend(function.docs().split("\n"));
                }

                builder.extend(function.args().flat_map(|(name, typ)| {
                    specta_typescript::datatype(
                        ts,
                        &FunctionResultVariant::Value(typ.clone()),
                        &cfg.type_map,
                    )
                    .map(|typ| {
                        let name = name.to_lower_camel_case();

                        format!("@param {{ {typ} }} {name}")
                    })
                }));
                builder.push(&format!("@returns {{ Promise<{ret_type}> }}"));

                builder.build()
            };

            Ok(js_ts::function(
                &jsdoc,
                &function.name().to_lower_camel_case(),
                // TODO: Don't `collect` the whole thing
                &js_ts::arg_names(&function.args().cloned().collect::<Vec<_>>()),
                None,
                &js_ts::command_body(&cfg.plugin_name, &function, false, cfg.error_handling),
            ))
        })
        .collect::<Result<Vec<_>, ExportError>>()?
        .join(",\n");

    Ok(format!(
        r#"export const commands = {{
        {commands}
    }}"#
    ))
}

fn render_query_keys(cfg: &ExportContext) -> Result<String, ExportError> {
    if cfg.queries.is_empty() {
        return Ok(Default::default());
    }

    let entries = cfg
        .queries
        .iter()
        .map(|function| {
            let name = function.name().to_lower_camel_case();
            let args: Vec<_> = function.args().cloned().collect();

            let key_prefix = if let Some(plugin_name) = cfg.plugin_name {
                format!("\"plugin:{plugin_name}\", \"{name}\"")
            } else {
                format!("\"{name}\"")
            };

            if args.is_empty() {
                format!("{name}: () => [{key_prefix}]")
            } else {
                let optional_args = args
                    .iter()
                    .map(|(n, _)| n.to_lower_camel_case())
                    .collect::<Vec<_>>()
                    .join(", ");

                format!(
                    "{name}: ({optional_args}) => filterKey([{key_prefix}], {{ {optional_args} }})"
                )
            }
        })
        .collect::<Vec<_>>()
        .join(",\n");

    Ok(format!(
        r#"
export const queryKeys = {{
    {entries}
}}"#
    ))
}

fn render_queries(cfg: &ExportContext) -> Result<String, ExportError> {
    if cfg.queries.is_empty() {
        return Ok(Default::default());
    }

    let entries = cfg
        .queries
        .iter()
        .map(|function| {
            let name = function.name().to_lower_camel_case();
            let args: Vec<_> = function.args().cloned().collect();

            let call_args = args
                .iter()
                .map(|(n, _)| n.to_lower_camel_case())
                .collect::<Vec<_>>()
                .join(", ");

            let arg_names_str = call_args.clone();

            let uses_typed_error = js_ts::command_uses_typed_error(function, cfg.error_handling);
            let query_fn_body = if uses_typed_error {
                format!("unwrapTypedError(commands.{name}({call_args}))")
            } else {
                format!("commands.{name}({call_args})")
            };

            format!(
                "{name}: ({arg_names_str}) => TANSTACK_QUERY_OPTIONS({{ queryKey: queryKeys.{name}({call_args}), queryFn: () => {query_fn_body} }})"
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    Ok(format!(
        r#"
export const queries = {{
    {entries}
}}"#
    ))
}

fn render_mutation_keys(cfg: &ExportContext) -> Result<String, ExportError> {
    if cfg.mutations.is_empty() {
        return Ok(Default::default());
    }

    let entries = cfg
        .mutations
        .iter()
        .map(|function| {
            let name = function.name().to_lower_camel_case();

            let key_prefix = if let Some(plugin_name) = cfg.plugin_name {
                format!("\"plugin:{plugin_name}\", \"{name}\"")
            } else {
                format!("\"{name}\"")
            };

            format!("{name}: () => [{key_prefix}]")
        })
        .collect::<Vec<_>>()
        .join(",\n");

    Ok(format!(
        r#"
export const mutationKeys = {{
    {entries}
}}"#
    ))
}

fn render_mutations(cfg: &ExportContext) -> Result<String, ExportError> {
    if cfg.mutations.is_empty() {
        return Ok(Default::default());
    }

    let entries = cfg
        .mutations
        .iter()
        .map(|function| {
            let name = function.name().to_lower_camel_case();
            let args: Vec<_> = function.args().cloned().collect();

            let call_args = args
                .iter()
                .map(|(n, _)| n.to_lower_camel_case())
                .collect::<Vec<_>>()
                .join(", ");

            let mutation_fn_param = if args.is_empty() {
                String::new()
            } else {
                format!("{{ {call_args} }}")
            };

            let uses_typed_error = js_ts::command_uses_typed_error(function, cfg.error_handling);
            let mutation_fn_body = if uses_typed_error {
                format!("unwrapTypedError(commands.{name}({call_args}))")
            } else {
                format!("commands.{name}({call_args})")
            };

            format!(
                "{name}: () => TANSTACK_MUTATION_OPTIONS({{ mutationKey: mutationKeys.{name}(), mutationFn: ({mutation_fn_param}) => {mutation_fn_body} }})"
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    Ok(format!(
        r#"
export const mutations = {{
    {entries}
}}"#
    ))
}

fn render_events(ts: &Typescript, cfg: &ExportContext) -> Result<String, ExportError> {
    if cfg.events.is_empty() {
        return Ok(Default::default());
    }

    let (events_types, events_map) =
        js_ts::events_data(&cfg.events, ts, &cfg.plugin_name, &cfg.type_map)?;

    let events = {
        let mut builder = js_doc::Builder::default();

        builder.push("@type {typeof __makeEvents__<{");
        builder.extend(events_types);
        builder.push("}>}");

        builder.build()
    };

    Ok(format! {
        r#"
    {events}
    const __typedMakeEvents__ = __makeEvents__;

    export const events = __typedMakeEvents__({{
    {events_map}
    }})"#
    })
}
