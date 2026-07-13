use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::Value as JsonValue;

use super::ast::*;
use super::*;

pub fn export_empty_sprite(name: &str, is_stage: bool) -> HashMap<String, JsonValue> {
    let mut res = HashMap::new();
    res.insert("isStage".to_string(), JsonValue::Bool(is_stage));
    res.insert(
        "name".to_string(),
        JsonValue::String(if is_stage { "Stage".to_string() } else { name.to_string() }),
    );
    res.insert("variables".to_string(), serde_json::json!({}));
    res.insert("lists".to_string(), serde_json::json!({}));
    res.insert("broadcasts".to_string(), serde_json::json!({}));
    res.insert("blocks".to_string(), serde_json::json!({}));
    res.insert("comments".to_string(), serde_json::json!({}));
    res.insert("currentCostume".to_string(), JsonValue::Number(0.into()));
    res.insert("costumes".to_string(), serde_json::json!([make_empty_costume("")]));
    res.insert("sounds".to_string(), serde_json::json!([]));
    res.insert("volume".to_string(), JsonValue::Number(100.into()));
    res.insert(
        "layerOrder".to_string(),
        JsonValue::Number(if is_stage { 0 } else { 1 }.into()),
    );
    res.insert("visible".to_string(), JsonValue::Bool(true));

    if !is_stage {
        res.insert("x".to_string(), JsonValue::Number(0.into()));
        res.insert("y".to_string(), JsonValue::Number(0.into()));
        res.insert("size".to_string(), JsonValue::Number(100.into()));
        res.insert("direction".to_string(), JsonValue::Number(90.into()));
        res.insert("draggable".to_string(), JsonValue::Bool(false));
        res.insert(
            "rotationStyle".to_string(),
            JsonValue::String("all around".to_string()),
        );
    } else {
        res.insert("tempo".to_string(), JsonValue::Number(60.into()));
        res.insert("videoTransparency".to_string(), JsonValue::Number(50.into()));
        res.insert(
            "videoState".to_string(),
            JsonValue::String("on".to_string()),
        );
        res.insert("textToSpeechLanguage".to_string(), JsonValue::Null);
    }

    res
}

pub fn export_data(ctx: &mut ScratchContext, format: Format) -> String {
    let mut sprite = export_empty_sprite(MAIN_SPRITE_NAME, false);
    let raw = ctx_get_raw(ctx);
    for (k, v) in raw {
        sprite.insert(k, v);
    }

    match format {
        Format::Sprite3 => serde_json::to_string(&sprite).unwrap_or_default(),
        Format::Project3 => {
            let buffer_sprite = export_empty_sprite(EMPTY_SPRITE_NAME, false);

            let empty_sprite_comment = format!(
                "WARNING: The '{}' sprite may contain a lot of blocks and cause the scratch editor to crash! \
                 Make a backup of the project before opening! Also, opening it may cause any project.json tweaks enabled \
                 to break (not all projects use these so it should be fine).\n\n\
                 This project was compiled from C, C++, Rust or other languages using llvm2scratch. The author of the \
                 project should have included the source code used to compile it, so check the project description! \
                 If you really want to read the generated scratch code (which is quite difficult to understand), the \
                 author may have also provided a text version.",
                MAIN_SPRITE_NAME
            );

            let mut buffer_with_comment = buffer_sprite;
            buffer_with_comment.insert(
                "comments".to_string(),
                serde_json::json!({
                    "coolcommentid": {
                        "blockId": null,
                        "x": 50,
                        "y": 50,
                        "width": 500,
                        "height": 300,
                        "minimized": false,
                        "text": empty_sprite_comment
                    }
                }),
            );

            let stage = export_empty_sprite("", true);

            let project = serde_json::json!({
                "targets": [stage, buffer_with_comment, sprite],
                "monitors": [],
                "extensions": [],
                "meta": {
                    "semver": "3.0.0",
                    "vm": "13.6.10",
                    "agent": "Project compiled with llvm2scratch!"
                }
            });

            serde_json::to_string(&project).unwrap_or_default()
        }
    }
}

fn ctx_get_raw(ctx: &mut ScratchContext) -> HashMap<String, JsonValue> {
    while !ctx.late_blocks.is_empty() {
        let late = ctx.late_blocks.drain(..).collect::<Vec<_>>();
        for (id, block_data, meta) in late {
            match block_data {
                LateBlockData::ProcedureCall(data) => {
                    let mut values = Vec::new();
                    for arg in &data.args {
                        let val = get_raw_value(arg, &id, ctx, ScratchCast::ToStr);
                        values.push(val);
                    }
                    let (param_ids, run_without_refresh) = ctx.funcs.get(&data.name)
                        .cloned()
                        .unwrap_or_default();

                    let mut inputs = serde_json::Map::new();
                    for (param_id, val) in param_ids.iter().zip(values.iter()) {
                        inputs.insert(param_id.clone(), serde_json::json!(val));
                    }

                    let proccode = format!("{}{}", sanitize_proc_name(&data.name, false), " %s".repeat(param_ids.len()));

                    let raw_block = {
                        let mut r = HashMap::new();
                        r.insert("opcode".to_string(), JsonValue::String("procedures_call".to_string()));
                        if !inputs.is_empty() {
                            r.insert("inputs".to_string(), JsonValue::Object(inputs));
                        }
                        r.insert("mutation".to_string(), serde_json::json!({
                            "tagName": "mutation",
                            "children": [],
                            "proccode": proccode,
                            "argumentids": serde_json::to_string(&param_ids).unwrap_or_default(),
                            "warp": serde_json::to_string(&run_without_refresh).unwrap_or_default()
                        }));
                        r
                    };
                    ctx.add_block(&id, &Block::RawBlock(raw_block), &meta);
                }
            }
        }
    }
    let mut raw_vars = serde_json::Map::new();
    for (name, (id, value)) in &ctx.vars {
        raw_vars.insert(id.clone(), serde_json::json!([name, get_raw_var_init(value)]));
    }

    let mut raw_lists = serde_json::Map::new();
    for (name, (id, values)) in &ctx.lists {
        let raw_vals: Vec<JsonValue> = values
            .iter()
            .map(|v| serde_json::Value::String(get_raw_var_init(v)))
            .collect();
        raw_lists.insert(id.clone(), serde_json::json!([name, raw_vals]));
    }

    let mut raw_broadcasts = serde_json::Map::new();
    for (name, id) in &ctx.broadcasts {
        raw_broadcasts.insert(id.clone(), serde_json::Value::String(name.clone()));
    }

    let mut raw_blocks = serde_json::Map::new();
    for (id, block) in &ctx.blocks {
        raw_blocks.insert(id.clone(), serde_json::Value::Object(block.iter().map(|(k, v)| (k.clone(), v.clone())).collect()));
    }

    let mut raw_costumes = Vec::new();
    let costume_names = if ctx.costumes.is_empty() {
        vec!["".to_string()]
    } else {
        ctx.costumes.clone()
    };
    for name in &costume_names {
        raw_costumes.push(serde_json::to_value(make_empty_costume(name)).unwrap());
    }

    let mut result = HashMap::new();
    result.insert("variables".to_string(), JsonValue::Object(raw_vars));
    result.insert("lists".to_string(), JsonValue::Object(raw_lists));
    result.insert("broadcasts".to_string(), JsonValue::Object(raw_broadcasts));
    result.insert("blocks".to_string(), JsonValue::Object(raw_blocks));
    result.insert("costumes".to_string(), JsonValue::Array(raw_costumes));
    result
}

fn get_raw_var_init(val: &KnownVal) -> String {
    match val {
        KnownVal::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        KnownVal::Str(s) => s.clone(),
        KnownVal::Num(n) => {
            if n.is_infinite() && *n > 0.0 {
                "Infinity".to_string()
            } else if n.is_infinite() && *n < 0.0 {
                "-Infinity".to_string()
            } else if n.is_nan() {
                "NaN".to_string()
            } else if *n == 0.0 && n.is_sign_negative() {
                "-0".to_string()
            } else if n.fract() == 0.0 {
                (*n as i64).to_string()
            } else {
                n.to_string()
            }
        }
    }
}

pub fn export_scratch_file(ctx: &mut ScratchContext, path: &str, format: Format) -> std::io::Result<()> {
    let (folder, file) = match format {
        Format::Project3 => ("Project", "project.json"),
        Format::Sprite3 => ("Sprite", "sprite.json"),
    };

    let data = export_data(ctx, format);
    let svg_hash = empty_svg_hash();

    let file_path = Path::new(path);
    let _file_stem = file_path.file_stem().unwrap().to_str().unwrap();
    let zip_path = file_path.with_extension(match format {
        Format::Project3 => "sb3",
        Format::Sprite3 => "sprite3",
    });

    let zip_file = fs::File::create(&zip_path)?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file(format!("{}/{}", folder, file), options)?;
    zip.write_all(data.as_bytes())?;

    zip.start_file(format!("{}/{}.svg", folder, svg_hash), options)?;
    zip.write_all(EMPTY_SVG.as_bytes())?;

    zip.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_empty_sprite() {
        let sprite = export_empty_sprite("Test", false);
        assert_eq!(sprite["name"], JsonValue::String("Test".to_string()));
        assert_eq!(sprite["isStage"], JsonValue::Bool(false));
    }

    #[test]
    fn test_export_empty_sprite_stage() {
        let stage = export_empty_sprite("", true);
        assert_eq!(stage["name"], JsonValue::String("Stage".to_string()));
        assert_eq!(stage["isStage"], JsonValue::Bool(true));
    }

    #[test]
    fn test_get_raw_var_init() {
        assert_eq!(get_raw_var_init(&KnownVal::Num(0.0)), "0");
        assert_eq!(get_raw_var_init(&KnownVal::Num(-0.0)), "-0");
        assert_eq!(get_raw_var_init(&KnownVal::Num(42.0)), "42");
        assert_eq!(get_raw_var_init(&KnownVal::Bool(true)), "true");
        assert_eq!(get_raw_var_init(&KnownVal::Bool(false)), "false");
        assert_eq!(get_raw_var_init(&KnownVal::Str("hello".to_string())), "hello");
    }

    #[test]
    fn test_export_data_sprite3() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let data = export_data(&mut ctx, Format::Sprite3);
        assert!(!data.is_empty());
        let parsed: JsonValue = serde_json::from_str(&data).unwrap();
        assert!(parsed.is_object());
    }

    #[test]
    fn test_export_data_project3() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let data = export_data(&mut ctx, Format::Project3);
        assert!(!data.is_empty());
        let parsed: JsonValue = serde_json::from_str(&data).unwrap();
        assert!(parsed.is_object());
        assert!(parsed.as_object().unwrap().contains_key("targets"));
    }

    #[test]
    fn test_export_data_with_blocks() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let bl = BlockList::from_blocks(vec![
            Block::OnStartFlag,
            Block::Say { value: Value::Known(KnownVal::Str("Hello!".to_string())) },
        ]);
        ctx.add_block_list(&bl, None);
        let data = export_data(&mut ctx, Format::Project3);
        let parsed: JsonValue = serde_json::from_str(&data).unwrap();
        let targets = parsed["targets"].as_array().unwrap();
        let sprite = &targets[2];
        let blocks = sprite["blocks"].as_object().unwrap();
        assert!(!blocks.is_empty());
    }

    #[test]
    fn test_export_data_with_vars() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let var_id = ctx.add_or_get_var("x", Some(KnownVal::Num(42.0)));
        let data = export_data(&mut ctx, Format::Project3);
        let parsed: JsonValue = serde_json::from_str(&data).unwrap();
        let targets = parsed["targets"].as_array().unwrap();
        let sprite = &targets[2];
        let variables = sprite["variables"].as_object().unwrap();
        assert!(variables.contains_key(&var_id));
    }

    #[test]
    fn test_export_data_with_lists() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let list_id = ctx.add_or_get_list("mylist", vec![KnownVal::Num(1.0), KnownVal::Num(2.0)]);
        let data = export_data(&mut ctx, Format::Project3);
        let parsed: JsonValue = serde_json::from_str(&data).unwrap();
        let targets = parsed["targets"].as_array().unwrap();
        let sprite = &targets[2];
        let lists = sprite["lists"].as_object().unwrap();
        assert!(lists.contains_key(&list_id));
    }

    #[test]
    fn test_export_data_with_broadcasts() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let broadcast_id = ctx.add_broadcast("my_event");
        let data = export_data(&mut ctx, Format::Project3);
        let parsed: JsonValue = serde_json::from_str(&data).unwrap();
        let targets = parsed["targets"].as_array().unwrap();
        let sprite = &targets[2];
        let broadcasts = sprite["broadcasts"].as_object().unwrap();
        assert!(!broadcasts.is_empty());
        assert!(broadcasts.contains_key(&broadcast_id));
    }

    #[test]
    fn test_export_scratch_file_to_disk() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let bl = BlockList::from_blocks(vec![
            Block::OnStartFlag,
            Block::Say { value: Value::Known(KnownVal::Str("Test export".to_string())) },
        ]);
        ctx.add_block_list(&bl, None);
        let tmp_dir = std::env::temp_dir();
        let path = tmp_dir.join("test_export").to_str().unwrap().to_string();
        let result = export_scratch_file(&mut ctx, &path, Format::Project3);
        assert!(result.is_ok());
        let sb3_path = tmp_dir.join("test_export.sb3");
        assert!(sb3_path.exists());
        let _ = std::fs::remove_file(&sb3_path);
    }
}