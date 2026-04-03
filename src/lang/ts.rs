use std::path::Path;

use crate::{lang::js_ts, ExportContext, LanguageExt};
use heck::ToLowerCamelCase;
use specta::datatype::FunctionResultVariant;
use specta_typescript::{self as ts, Typescript};
use specta_typescript::{js_doc, ExportError};

const GLOBALS: &str = include_str!("./globals.ts");

impl LanguageExt for specta_typescript::Typescript {
    type Error = ExportError;

    fn render(&self, cfg: &ExportContext) -> Result<String, ExportError> {
        let dependant_types = cfg
            .type_map
            .into_iter()
            .map(|(_sid, ndt)| ts::export_named_datatype(&self, ndt, &cfg.type_map))
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join("\n"))?;

        let tanstack_import = super::js::render_tanstack_import(cfg);

        let header = if tanstack_import.is_empty() {
            self.header.to_string()
        } else {
            format!("{}\n{tanstack_import}", self.header)
        };

        js_ts::render_all_parts::<Self>(
            cfg,
            &dependant_types,
            GLOBALS,
            &header,
            render_commands(self, cfg)?,
            render_query_keys(self, cfg)?,
            render_queries(self, cfg)?,
            render_mutation_keys(self, cfg)?,
            render_mutations(self, cfg)?,
            render_events(self, cfg)?,
            true,
        )
    }

    fn format(&self, path: &Path) -> Result<(), Self::Error> {
        if let Some(formatter) = self.formatter {
            formatter(path)?;
        }
        Ok(())
    }
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
            let arg_defs = function
                .args()
                .map(|(name, typ)| {
                    ts::datatype(
                        ts,
                        &FunctionResultVariant::Value(typ.clone()),
                        &cfg.type_map,
                    )
                    .map(|ty| format!("{}: {}", name.to_lower_camel_case(), ty))
                })
                .collect::<Result<Vec<_>, _>>()?;

            let ret_type = js_ts::handle_result(function, &cfg.type_map, ts, cfg.error_handling)?;

            let docs = {
                let mut builder = js_doc::Builder::default();

                if let Some(d) = &function.deprecated() {
                    builder.push_deprecated(d);
                }

                if !function.docs().is_empty() {
                    builder.extend(function.docs().split("\n"));
                }

                builder.build()
            };
            Ok(js_ts::function(
                &docs,
                &function.name().to_lower_camel_case(),
                &arg_defs,
                Some(&ret_type),
                &js_ts::command_body(&cfg.plugin_name, function, true, cfg.error_handling),
            ))
        })
        .collect::<Result<Vec<_>, ExportError>>()?
        .join(",\n");

    Ok(format! {
        r#"
export const commands = {{
{commands}
}}"#
    })
}

fn render_query_keys(ts: &Typescript, cfg: &ExportContext) -> Result<String, ExportError> {
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
                Ok(format!("{name}: () => [{key_prefix}] as const"))
            } else {
                let optional_args = args
                    .iter()
                    .map(|(arg_name, typ)| {
                        let ty = ts::datatype(
                            ts,
                            &FunctionResultVariant::Value(typ.clone()),
                            &cfg.type_map,
                        )?;
                        Ok(format!("{}?: {}", arg_name.to_lower_camel_case(), ty))
                    })
                    .collect::<Result<Vec<_>, ExportError>>()?
                    .join(", ");

                let args_obj = args
                    .iter()
                    .map(|(n, _)| n.to_lower_camel_case())
                    .collect::<Vec<_>>()
                    .join(", ");

                Ok(format!(
                    "{name}: ({optional_args}) => filterKey([{key_prefix}] as const, {{ {args_obj} }})"
                ))
            }
        })
        .collect::<Result<Vec<_>, ExportError>>()?
        .join(",\n");

    Ok(format!(
        r#"
export const queryKeys = {{
    {entries}
}}"#
    ))
}

fn render_queries(ts: &Typescript, cfg: &ExportContext) -> Result<String, ExportError> {
    if cfg.queries.is_empty() {
        return Ok(Default::default());
    }

    let entries = cfg
        .queries
        .iter()
        .map(|function| {
            let name = function.name().to_lower_camel_case();
            let args: Vec<_> = function.args().cloned().collect();

            let arg_defs = args
                .iter()
                .map(|(arg_name, typ)| {
                    let ty =
                        ts::datatype(ts, &FunctionResultVariant::Value(typ.clone()), &cfg.type_map)?;
                    Ok(format!("{}: {}", arg_name.to_lower_camel_case(), ty))
                })
                .collect::<Result<Vec<_>, ExportError>>()?
                .join(", ");

            let call_args = args
                .iter()
                .map(|(n, _)| n.to_lower_camel_case())
                .collect::<Vec<_>>()
                .join(", ");

            let (data_type, error_type) =
                js_ts::extract_tanstack_result_types(function, ts, &cfg.type_map)?;

            let generics = match &error_type {
                Some(e) => format!("<{data_type}, {e}>"),
                None => format!("<{data_type}>"),
            };

            let uses_typed_error = js_ts::command_uses_typed_error(function, cfg.error_handling);
            let query_fn_body = if uses_typed_error {
                format!("unwrapTypedError(commands.{name}({call_args}))")
            } else {
                format!("commands.{name}({call_args})")
            };

            Ok(format!(
                "{name}: ({arg_defs}) => TANSTACK_QUERY_OPTIONS{generics}({{ queryKey: queryKeys.{name}({call_args}), queryFn: () => {query_fn_body} }})"
            ))
        })
        .collect::<Result<Vec<_>, ExportError>>()?
        .join(",\n");

    Ok(format!(
        r#"
export const queries = {{
    {entries}
}}"#
    ))
}

fn render_mutation_keys(_ts: &Typescript, cfg: &ExportContext) -> Result<String, ExportError> {
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

            format!("{name}: () => [{key_prefix}] as const")
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

fn render_mutations(ts: &Typescript, cfg: &ExportContext) -> Result<String, ExportError> {
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

            let (data_type, error_type) =
                js_ts::extract_tanstack_result_types(function, ts, &cfg.type_map)?;

            let variables_type = if args.is_empty() {
                "void".to_string()
            } else {
                let fields = args
                    .iter()
                    .map(|(arg_name, typ)| {
                        let ty = ts::datatype(
                            ts,
                            &FunctionResultVariant::Value(typ.clone()),
                            &cfg.type_map,
                        )?;
                        Ok(format!("{}: {}", arg_name.to_lower_camel_case(), ty))
                    })
                    .collect::<Result<Vec<_>, ExportError>>()?
                    .join(", ");
                format!("{{ {fields} }}")
            };

            let generics = match &error_type {
                Some(e) => format!("<{data_type}, {e}, {variables_type}>"),
                None => format!("<{data_type}, Error, {variables_type}>"),
            };

            let mutation_fn_param = if args.is_empty() {
                String::new()
            } else {
                format!("{{ {call_args} }}: {variables_type}")
            };

            let uses_typed_error = js_ts::command_uses_typed_error(function, cfg.error_handling);
            let mutation_fn_body = if uses_typed_error {
                format!("unwrapTypedError(commands.{name}({call_args}))")
            } else {
                format!("commands.{name}({call_args})")
            };

            Ok(format!(
                "{name}: () => TANSTACK_MUTATION_OPTIONS{generics}({{ mutationKey: mutationKeys.{name}(), mutationFn: ({mutation_fn_param}) => {mutation_fn_body} }})"
            ))
        })
        .collect::<Result<Vec<_>, ExportError>>()?
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

    let events_types = events_types.join(",\n");

    Ok(format! {
        r#"
export const events = __makeEvents__<{{
{events_types}
}}>({{
{events_map}
}})"#
    })
}
