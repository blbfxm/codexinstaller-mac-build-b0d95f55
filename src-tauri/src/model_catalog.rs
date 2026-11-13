use serde_json::{json, Value};

// Older desktop engines may not support `debug models --bundled`. Their
// persisted catalog still supplies the version-matched instructions/schema.
pub(crate) fn from_cache(home: &std::path::Path, model_ids: &[String]) -> Result<Value, String> {
    for name in ["models_cache.json", "model-catalog.json"] {
        let catalog = std::fs::read(home.join(name)).ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .and_then(|catalog| merge_upstream_models(catalog, model_ids).ok());
        if let Some(catalog) = catalog { return Ok(catalog); }
    }
    Err("无法读取 Codex 模型目录。请先打开 Codex 完成首次引导后退出，再重试一键配置；若仍失败，请更新 Codex。未应用供应商配置。".into())
}

// A freshly installed desktop build may not have launched once, so neither
// its bundled engine nor its persisted catalog is available yet. Codex still
// needs the same minimal model envelope to accept provider configuration.
pub(crate) fn fallback_catalog(model_ids: &[String]) -> Result<Value, String> {
    const INSTRUCTIONS: &str = "You are Codex, a coding agent working with the user in their workspace. Help inspect, edit, run, and verify code. Use the available tools when they are needed, keep changes scoped to the request, and explain material risks or failed checks. Preserve unrelated user changes and do not claim work is complete until it has been verified.";

    let mut models = Vec::new();
    for model_id in model_ids {
        let model_id = model_id.trim();
        if model_id.is_empty() || models.iter().any(|model: &Value| model["slug"] == model_id) {
            continue;
        }
        models.push(json!({
            "slug": model_id,
            "display_name": model_id,
            "description": format!("{model_id}（坚果供应商）"),
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [
                {"effort": "low", "description": "Low"},
                {"effort": "medium", "description": "Medium"},
                {"effort": "high", "description": "High"}
            ],
            "shell_type": "unified_exec",
            "visibility": "list",
            "supported_in_api": true,
            "priority": models.len() + 1,
            "additional_speed_tiers": [],
            "service_tiers": [],
            "default_service_tier": null,
            "available_access_programs": null,
            "upgrade": null,
            "base_instructions": INSTRUCTIONS,
            "model_messages": {
                "instructions_template": INSTRUCTIONS,
                "instructions_variables": {}
            },
            "include_skills_usage_instructions": false,
            "include_plugin_usage_instructions": false,
            "include_apps_usage_instructions": false,
            "supports_reasoning_summaries": true,
            "supports_reasoning_summary_parameter": true,
            "default_reasoning_summary": "none",
            "support_verbosity": true,
            "default_verbosity": "low",
            "apply_patch_tool_type": "freeform",
            "web_search_tool_type": "text_and_image",
            "truncation_policy": {"mode": "tokens", "limit": 10000},
            "supports_parallel_tool_calls": true,
            "supports_image_detail_original": false,
            "context_window": 128000,
            "max_context_window": 128000,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text"],
            "supports_search_tool": false,
            "use_responses_lite": false
        }));
    }
    if models.is_empty() {
        return Err("模型目录为空，无法配置 Codex 模型选择器".into());
    }
    Ok(json!({ "models": models }))
}

pub(crate) fn merge_upstream_models(
    mut catalog: Value,
    model_ids: &[String],
) -> Result<Value, String> {
    let models = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or("Codex 引擎返回的模型目录格式无效")?;
    let template = models
        .iter()
        .find(|model| model["supported_in_api"].as_bool() == Some(true) && has_instructions(model))
        .or_else(|| models.iter().find(|model| has_instructions(model)))
        .cloned()
        .ok_or("Codex 引擎的模型目录为空")?;

    let mut added = 0usize;
    for model_id in model_ids {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            continue;
        }
        let existing_index = models
            .iter()
            .position(|model| model["slug"].as_str() == Some(model_id));
        if existing_index.is_some_and(|index| has_instructions(&models[index])) {
            let model = &mut models[existing_index.unwrap()];
            model["visibility"] = json!("list");
            model["supported_in_api"] = json!(true);
            continue;
        }

        let priority = 1000 + added;
        let mut model = template.clone();
        model["slug"] = json!(model_id);
        model["display_name"] = json!(model_id);
        model["description"] = json!(format!("{model_id}（坚果供应商）"));
        model["default_reasoning_level"] = json!("medium");
        model["supported_reasoning_levels"] = json!([
            {"effort": "low", "description": "Low"},
            {"effort": "medium", "description": "Medium"},
            {"effort": "high", "description": "High"}
        ]);
        model["visibility"] = json!("list");
        model["supported_in_api"] = json!(true);
        model["priority"] = json!(priority);
        for field in [
            "availability_nux",
            "upgrade",
            "service_tiers",
            "additional_speed_tiers",
        ] {
            if let Some(object) = model.as_object_mut() {
                object.remove(field);
            }
        }
        if let Some(index) = existing_index {
            models[index] = model;
        } else {
            models.push(model);
        }
        added += 1;
    }

    Ok(catalog)
}

fn has_instructions(model: &Value) -> bool {
    [model.get("base_instructions"), model.pointer("/model_messages/instructions_template")]
        .into_iter().flatten().any(|value| value.as_str().is_some_and(|text| !text.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_native_catalog_recovers_when_engine_export_is_unavailable() {
        let home = std::env::temp_dir().join(format!("model-catalog-cache-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        assert!(from_cache(&home, &["custom-model".into()]).is_err());
        std::fs::write(home.join("model-catalog.json"), b"invalid").unwrap();
        let native = json!({"models":[{
            "slug":"native-model", "visibility":"hide", "supported_in_api":false,
            "base_instructions":"Native version instructions"
        }]});
        std::fs::write(home.join("models_cache.json"), serde_json::to_vec(&native).unwrap()).unwrap();
        let ids = vec!["native-model".into(), "custom-model".into()];
        let catalog = from_cache(&home, &ids).unwrap();
        assert_eq!(catalog["models"][0]["visibility"], "list");
        assert_eq!(catalog["models"][0]["supported_in_api"], true);
        assert_eq!(catalog["models"][1]["slug"], "custom-model");
        assert_eq!(catalog["models"][1]["base_instructions"], "Native version instructions");
        std::fs::write(home.join("model-catalog.json"), serde_json::to_vec(&catalog).unwrap()).unwrap();
        std::fs::remove_file(home.join("models_cache.json")).unwrap();
        assert_eq!(from_cache(&home, &ids).unwrap(), catalog);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn fallback_catalog_supports_fresh_install_without_native_cache() {
        let catalog = fallback_catalog(&[
            "deepseek-chat".into(),
            "  kimi-k2  ".into(),
            "deepseek-chat".into(),
            "".into(),
        ])
        .unwrap();
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "deepseek-chat");
        assert_eq!(models[1]["slug"], "kimi-k2");
        assert!(models.iter().all(|model| has_instructions(model)));
        assert!(models
            .iter()
            .all(|model| model["model_messages"]["instructions_template"]
                .as_str()
                .is_some_and(|text| !text.trim().is_empty())));
    }

    #[test]
    fn fallback_catalog_rejects_an_empty_model_list() {
        assert!(fallback_catalog(&[]).is_err());
        assert!(fallback_catalog(&["  ".into()]).is_err());
    }

    #[test]
    fn merges_upstream_models_into_native_catalog_without_duplicate_slugs() {
        let catalog = json!({
            "models": [{
                "slug": "gpt-5",
                "display_name": "GPT-5",
                "description": "Built in",
                "default_reasoning_level": "high",
                "supported_reasoning_levels": [{"effort": "high", "description": "High"}],
                "visibility": "list",
                "supported_in_api": true,
                "priority": 1,
                "model_messages": {
                    "instructions_template": "Use the Codex agent instructions supplied with the request.",
                    "persistent_instructions": "Runtime-specific instructions"
                },
                "base_instructions": "Use the Codex agent instructions supplied with the request."
            }]
        });
        let result = merge_upstream_models(
            catalog,
            &[
                "gpt-5".into(),
                "deepseek-chat".into(),
                "kimi-k2".into(),
                "  ".into(),
            ],
        )
        .unwrap();

        let models = result["models"].as_array().unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0]["slug"], "gpt-5");
        assert_eq!(models[1]["slug"], "deepseek-chat");
        assert_eq!(models[1]["visibility"], "list");
        assert_eq!(models[1]["supported_in_api"], true);
        assert_eq!(models[1]["default_reasoning_level"], "medium");
        assert_eq!(
            models[1]["model_messages"]["instructions_template"],
            "Use the Codex agent instructions supplied with the request."
        );
        assert_eq!(
            models[1]["base_instructions"],
            "Use the Codex agent instructions supplied with the request."
        );
        assert_eq!(
            models[1]["model_messages"]["persistent_instructions"],
            "Runtime-specific instructions"
        );
        assert_eq!(models[2]["slug"], "kimi-k2");
        assert!(models
            .iter()
            .all(|model| model["priority"].as_u64().is_some()));
    }

    #[test]
    fn rejects_catalog_without_models() {
        assert!(merge_upstream_models(json!({}), &["test".into()]).is_err());
        assert!(merge_upstream_models(json!({"models": []}), &["test".into()]).is_err());
        assert!(merge_upstream_models(
            json!({"models": [{"slug": "gpt", "supported_in_api": true}]}),
            &["test".into()]
        )
        .is_err());
        assert!(merge_upstream_models(json!({"models":[{
            "slug":"broken", "base_instructions":null,
            "model_messages":{"instructions_template":" "}
        }]}), &["test".into()]).is_err());
    }

    #[test]
    fn repairs_previous_custom_entries_missing_required_instructions() {
        let catalog = json!({
            "models": [
                {
                    "slug": "gpt-5",
                    "supported_in_api": true,
                    "base_instructions": "Codex instructions",
                    "model_messages": {"instructions_template": "Codex template"}
                },
                {
                    "slug": "claude-opus-5-5",
                    "supported_in_api": true,
                    "visibility": "list"
                }
            ]
        });
        let result =
            merge_upstream_models(catalog, &["claude-opus-5-5".into(), "deepseek-chat".into()])
                .unwrap();
        let models = result["models"].as_array().unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models[1]["base_instructions"], "Codex instructions");
        assert_eq!(
            models[1]["model_messages"]["instructions_template"],
            "Codex template"
        );
        assert_eq!(models[2]["slug"], "deepseek-chat");
        assert_eq!(models[2]["base_instructions"], "Codex instructions");
    }

}
