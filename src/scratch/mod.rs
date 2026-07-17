pub mod ast;
pub mod export;
pub mod stringify;
pub mod uid;

pub use ast::*;

use std::collections::HashMap;

use serde_json::Value as JsonValue;

use uid::UidGenerator;

pub const MAIN_SPRITE_NAME: &str = "DONT OPEN";
pub const EMPTY_SPRITE_NAME: &str = "Empty";
pub const DEFAULT_BROADCAST_MESSAGE: &str = "message1";
pub const COUNTER_REPLACEMENT_NAME: &str = "!control:counter";

pub const EMPTY_SVG: &str = r#"<svg version="1.1" xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="0" height="0" viewBox="0,0,0,0"></svg>"#;

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub cfg: ScratchConfig,
    pub code: Vec<BlockList>,
    pub lists: HashMap<String, Vec<KnownVal>>,
    pub costumes: Vec<String>,
}

impl Project {
    pub fn new(cfg: ScratchConfig) -> Self {
        Project {
            cfg,
            code: Vec::new(),
            lists: HashMap::new(),
            costumes: Vec::new(),
        }
    }

    pub fn add_costume(&mut self, name: String) -> usize {
        self.costumes.push(name);
        self.costumes.len()
    }

    pub fn get_ctx(&self) -> ScratchContext {
        let mut ctx = ScratchContext::new(self.cfg.clone());
        for (name, scratch_list) in &self.lists {
            ctx.add_or_get_list(name, scratch_list.clone());
        }
        for block_list in &self.code {
            ctx.add_block_list(block_list, None);
        }
        ctx.costumes.extend(self.costumes.iter().cloned());
        ctx
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScratchContext {
    pub cfg: ScratchConfig,
    pub vars: HashMap<String, (Id, KnownVal)>,
    pub lists: HashMap<String, (Id, Vec<KnownVal>)>,
    pub broadcasts: HashMap<String, Id>,
    pub funcs: HashMap<String, (Vec<Id>, bool)>,
    pub blocks: HashMap<Id, HashMap<String, JsonValue>>,
    pub late_blocks: Vec<(Id, LateBlockData, BlockMeta)>,
    pub costumes: Vec<String>,
    uid_gen: UidGenerator,
    var_uid_gen: UidGenerator,
    pub exported: bool,
    pub uses_pen: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LateBlockData {
    ProcedureCall(ProcedureCallData),
}

impl ScratchContext {
    pub fn new(cfg: ScratchConfig) -> Self {
        let minify = cfg.minify;
        ScratchContext {
            cfg,
            vars: HashMap::new(),
            lists: HashMap::new(),
            broadcasts: HashMap::new(),
            funcs: HashMap::new(),
            blocks: HashMap::new(),
            late_blocks: Vec::new(),
            costumes: Vec::new(),
            uid_gen: UidGenerator::new(minify),
            var_uid_gen: UidGenerator::new(minify),
            exported: false,
            uses_pen: false,
        }
    }

    pub fn gen_id(&mut self) -> Id {
        self.uid_gen.gen_id()
    }

    pub fn gen_var_id(&mut self) -> Id {
        self.var_uid_gen.gen_id()
    }

    pub fn add_or_get_var(&mut self, var_name: &str, default_val: Option<KnownVal>) -> Id {
        let default = default_val.unwrap_or(KnownVal::Num(0.0));
        if let Some((id, _)) = self.vars.get(var_name) {
            id.clone()
        } else {
            let id = self.gen_id();
            self.vars.insert(var_name.to_string(), (id.clone(), default));
            id
        }
    }

    pub fn add_or_get_list(&mut self, list_name: &str, default_val: Vec<KnownVal>) -> Id {
        if let Some((id, existing)) = self.lists.get_mut(list_name) {
            if !default_val.is_empty() && !existing.is_empty() {
                panic!("List {} given default value twice", list_name);
            }
            if !default_val.is_empty() {
                *existing = default_val;
            }
            id.clone()
        } else {
            let id = self.gen_id();
            self.lists.insert(list_name.to_string(), (id.clone(), default_val));
            id
        }
    }

    pub fn add_func(&mut self, func_name: &str, param_ids: Vec<Id>, run_without_refresh: bool) {
        if self.funcs.contains_key(func_name) {
            panic!("Function {} registered twice", func_name);
        }
        self.funcs
            .insert(func_name.to_string(), (param_ids, run_without_refresh));
    }

    pub fn add_broadcast(&mut self, name: &str) -> Id {
        if let Some(id) = self.broadcasts.get(name) {
            return id.clone();
        }
        let id = self.gen_id();
        self.broadcasts.insert(name.to_string(), id.clone());
        id
    }

    pub fn add_block(&mut self, id: &Id, block: &Block, meta: &BlockMeta) {
        match block {
            Block::ProcedureCall(data) => {
                self.late_blocks.push((
                    id.clone(),
                    LateBlockData::ProcedureCall(data.clone()),
                    meta.clone(),
                ));
            }
            _ => {
                let mut raw = get_raw_block(block, id, self);
                if self.cfg.hide_blocks && meta.parent.is_none() {
                    meta_shadow(&mut raw);
                }
                meta.add_raw_meta(&mut raw, &self.cfg);
                self.blocks.insert(id.clone(), raw);
            }
        }
    }

    pub fn add_block_list(&mut self, blocks: &BlockList, parent: Option<&Id>) -> Option<Id> {
        if blocks.blocks.is_empty() {
            return None;
        }

        for block in &blocks.blocks {
            if block.needs_pen_extension() {
                self.uses_pen = true;
            }
        }

        let expanded = self.expand_for_each(blocks);

        let mut last_id: Option<Id> = None;
        let mut curr_id = self.gen_id();
        let first_id = curr_id.clone();
        let mut next_id = self.gen_id();
        let first_parent = parent.cloned();

        for (i, block) in expanded.blocks.iter().enumerate() {
            if i == expanded.blocks.len() - 1 {
                next_id = self.gen_id();
            }

            if let Some(ref _lid) = last_id
                && block.is_start()
            {
                panic!("Starting block has blocks before it");
            }

            let meta_parent = if i == 0 {
                first_parent.clone()
            } else {
                last_id.clone()
            };
            let meta = BlockMeta::new(meta_parent, if i < expanded.blocks.len() - 1 { Some(next_id.clone()) } else { None });

            self.add_block(&curr_id, block, &meta);

            last_id = Some(curr_id);
            curr_id = next_id.clone();
            next_id = self.gen_id();
        }

        Some(first_id)
    }

    fn expand_for_each(&self, blocks: &BlockList) -> BlockList {
        if self.cfg.allow_hacked_blocks {
            return blocks.clone();
        }

        let mut result = BlockList::new();
        let mut for_each_var_set = false;

        for block in &blocks.blocks {
            if let Block::ControlFlow(cf) = block
                && cf.op == ControlOp::ForEach {
                    if let Some(ref var) = cf.var
                        && !for_each_var_set {
                            result.add_block(Block::EditVar(EditVarData {
                                op: VarOp::Set,
                                name: var.clone(),
                                value: Value::Known(KnownVal::Num(0.0)),
                            }));
                        }
                    for_each_var_set = !for_each_var_set;
                }
            result.add_block(block.clone());
        }

        result
    }
}

fn meta_shadow(raw: &mut HashMap<String, JsonValue>) {
    raw.insert("shadow".to_string(), JsonValue::Bool(true));
}

fn get_raw_block(
    block: &Block,
    my_id: &Id,
    ctx: &mut ScratchContext,
) -> HashMap<String, JsonValue> {
    if let Block::RawBlock(contents) = block {
        return contents.clone();
    }

    let mut raw = HashMap::new();
    match block {
        Block::Say { value } => {
            raw.insert("opcode".to_string(), JsonValue::String("looks_say".to_string()));
            let input = get_raw_value(value, my_id, ctx, ScratchCast::ToStr);
            raw.insert("inputs".to_string(), serde_json::json!({"MESSAGE": input}));
        }
        Block::SwitchCostume { value } => {
            raw.insert("opcode".to_string(), JsonValue::String("looks_switchcostumeto".to_string()));
            match value {
                Value::Known(kv) => {
                    let name_str = scratch_cast_to_str(kv);
                    let proto_id = ctx.gen_id();
                    ctx.add_block(&proto_id, &Block::RawBlock({
                        let mut r = HashMap::new();
                        r.insert("opcode".to_string(), JsonValue::String("looks_costume".to_string()));
                        r.insert("fields".to_string(), serde_json::json!({"COSTUME": [name_str, null]}));
                        r
                    }), &BlockMeta::new_shadow(Some(my_id.clone()), None));
                    raw.insert("inputs".to_string(), serde_json::json!({"COSTUME": [1, proto_id]}));
                }
                _ => {
                    let input = get_raw_value(value, my_id, ctx, ScratchCast::ToStr);
                    raw.insert("inputs".to_string(), serde_json::json!({"COSTUME": input}));
                }
            }
        }
        Block::EditVolume { op, value } => {
            let opcode = match op {
                VolumeOp::Set => "sound_setvolumeto",
                VolumeOp::Change => "sound_changevolumeby",
            };
            raw.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
            let input = get_raw_value(value, my_id, ctx, ScratchCast::ToStr);
            raw.insert("inputs".to_string(), serde_json::json!({"VOLUME": input}));
        }
        Block::Broadcast { value, wait } => {
            let opcode = if *wait { "event_broadcastandwait" } else { "event_broadcast" };
            raw.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));

            let broadcast_name = match value {
                Value::Known(kv) => scratch_cast_to_str(kv),
                _ => DEFAULT_BROADCAST_MESSAGE.to_string(),
            };
            let broadcast_id = ctx.add_broadcast(&broadcast_name);

            match value {
                Value::Known(_) => {
                    raw.insert("inputs".to_string(), serde_json::json!({
                        "BROADCAST_INPUT": [1, [11, broadcast_name, broadcast_id]]
                    }));
                }
                _ => {
                    let raw_input = get_raw_value(value, my_id, ctx, ScratchCast::ToStr);
                    raw.insert("inputs".to_string(), serde_json::json!({
                        "BROADCAST_INPUT": [3, raw_input[1].clone(), [11, broadcast_name, broadcast_id]]
                    }));
                }
            }
        }
        Block::OnBroadcast { name } => {
            raw.insert("opcode".to_string(), JsonValue::String("event_whenbroadcastreceived".to_string()));
            let broadcast_id = ctx.add_broadcast(name);
            raw.insert("fields".to_string(), serde_json::json!({"BROADCAST_OPTION": [name, broadcast_id]}));
        }
        Block::OnStartFlag => {
            raw.insert("opcode".to_string(), JsonValue::String("event_whenflagclicked".to_string()));
        }
        Block::ControlFlow(cf) => {
            let (op, val, body, else_body, var) = (&cf.op, &cf.condition, &cf.body, &cf.else_body, &cf.var);

            let effective_op = if !ctx.cfg.allow_hacked_blocks {
                match op {
                    ControlOp::While => ControlOp::Until,
                    other => *other,
                }
            } else {
                *op
            };

            let opcode = opcode_from_short_op(match effective_op {
                ControlOp::If => "if",
                ControlOp::IfElse => "if_else",
                ControlOp::RepTimes => "reptimes",
                ControlOp::Until => "until",
                ControlOp::While => "while",
                ControlOp::Forever => "forever",
                ControlOp::ForEach => "for_each",
            }).unwrap_or("control_if");

            raw.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));

            let effective_body = if !ctx.cfg.allow_hacked_blocks && *op == ControlOp::ForEach {
                let mut new_blocks = Vec::new();
                if let Some(var_name) = var {
                    new_blocks.push(Block::EditVar(EditVarData {
                        op: VarOp::Change,
                        name: var_name.clone(),
                        value: Value::Known(KnownVal::Num(1.0)),
                    }));
                }
                if let Some(b) = body {
                    new_blocks.extend(b.blocks.iter().cloned());
                }
                BlockList::from_blocks(new_blocks)
            } else if let Some(b) = body {
                b.clone()
            } else {
                BlockList::new()
            };

            let blocks_id = ctx.add_block_list(&effective_body, Some(my_id));
            let mut inputs = serde_json::Map::new();
            if let Some(bid) = blocks_id {
                inputs.insert("SUBSTACK".to_string(), serde_json::json!([2, bid]));
            }

            if effective_op != ControlOp::Forever
                && let Some(cond_val) = val {
                    let effective_cond = if !ctx.cfg.allow_hacked_blocks && *op == ControlOp::While {
                        Value::BoolOp(BoolOp::Not(Box::new(cond_val.clone())))
                    } else {
                        cond_val.clone()
                    };

                    let input_name = match effective_op {
                        ControlOp::RepTimes => "TIMES",
                        ControlOp::ForEach => "VALUE",
                        _ => "CONDITION",
                    };

                    let raw_val = if matches!(effective_op, ControlOp::If | ControlOp::IfElse | ControlOp::Until | ControlOp::While) {
                        get_raw_bool_value(&effective_cond, my_id, ctx)
                    } else {
                        Some(serde_json::json!(get_raw_value(&effective_cond, my_id, ctx, ScratchCast::ToNum)))
                    };

                    if let Some(rv) = raw_val {
                        inputs.insert(input_name.to_string(), rv);
                    }
                }

            if effective_op == ControlOp::IfElse
                && let Some(else_b) = else_body {
                    let else_id = ctx.add_block_list(else_b, Some(my_id));
                    if let Some(eid) = else_id {
                        inputs.insert("SUBSTACK2".to_string(), serde_json::json!([2, eid]));
                    }
                }

            if !inputs.is_empty() {
                raw.insert("inputs".to_string(), JsonValue::Object(inputs));
            }

            if effective_op == ControlOp::ForEach
                && let Some(var_name) = var {
                    let var_id = ctx.add_or_get_var(var_name, None);
                    raw.insert("fields".to_string(), serde_json::json!({"VARIABLE": [var_name, var_id]}));
                }
        }
        Block::StopScript(opt) => {
            raw.insert("opcode".to_string(), JsonValue::String("control_stop".to_string()));
            let stop_val = match opt {
                StopOption::All => "all",
                StopOption::This => "this script",
                StopOption::Other => "other scripts in sprite",
            };
            raw.insert("fields".to_string(), serde_json::json!({"STOP_OPTION": [stop_val, null]}));
        }
        Block::EditCounter(op) => {
            if !ctx.cfg.allow_hacked_blocks {
                let (var_op, val) = match op {
                    CounterOp::Increment => (VarOp::Change, KnownVal::Num(1.0)),
                    CounterOp::Decrement => (VarOp::Change, KnownVal::Num(-1.0)),
                    CounterOp::Reset => (VarOp::Set, KnownVal::Num(0.0)),
                };
                return get_raw_block(
                    &Block::EditVar(EditVarData {
                        op: var_op,
                        name: COUNTER_REPLACEMENT_NAME.to_string(),
                        value: Value::Known(val),
                    }),
                    my_id,
                    ctx,
                );
            }
            let opcode = match op {
                CounterOp::Increment => "control_incr_counter",
                CounterOp::Decrement => "control_incr_counter",
                CounterOp::Reset => "control_clear_counter",
            };
            raw.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
        }
        Block::Wait { value } => {
            raw.insert("opcode".to_string(), JsonValue::String("control_wait".to_string()));
            let input = get_raw_value(value, my_id, ctx, ScratchCast::ToNum);
            raw.insert("inputs".to_string(), serde_json::json!({"DURATION": input}));
        }
        Block::Ask { value, .. } => {
            raw.insert("opcode".to_string(), JsonValue::String("sensing_askandwait".to_string()));
            let input = get_raw_value(value, my_id, ctx, ScratchCast::ToStr);
            raw.insert("inputs".to_string(), serde_json::json!({"QUESTION": input}));
        }
        Block::EditVar(data) => {
            let opcode = match data.op {
                VarOp::Set => "data_setvariableto",
                VarOp::Change => "data_changevariableby",
            };
            raw.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
            let var_id = ctx.add_or_get_var(&data.name, None);
            let cast = match data.op {
                VarOp::Set => ScratchCast::ToStr,
                VarOp::Change => ScratchCast::ToNum,
            };
            let input = get_raw_value(&data.value, my_id, ctx, cast);
            raw.insert("inputs".to_string(), serde_json::json!({"VALUE": input}));
            raw.insert("fields".to_string(), serde_json::json!({"VARIABLE": [data.name, var_id]}));
        }
        Block::EditList(data) => {
            let opcode = opcode_from_short_op(match data.op {
                ListEditOp::AddTo => "addto",
                ListEditOp::ReplaceAt => "replaceat",
                ListEditOp::InsertAt => "insertat",
                ListEditOp::DeleteAt => "deleteat",
                ListEditOp::DeleteAll => "deleteall",
            }).unwrap_or("data_addtolist");
            raw.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
            let list_id = ctx.add_or_get_list(&data.name, vec![]);
            let mut inputs = serde_json::Map::new();
            if let Some(ref index) = data.index {
                let raw_index = get_raw_value(index, my_id, ctx, ScratchCast::ToNum);
                inputs.insert("INDEX".to_string(), serde_json::json!(raw_index));
            }
            if let Some(ref item) = data.value {
                let raw_item = get_raw_value(item, my_id, ctx, ScratchCast::ToStr);
                inputs.insert("ITEM".to_string(), serde_json::json!(raw_item));
            }
            if !inputs.is_empty() {
                raw.insert("inputs".to_string(), JsonValue::Object(inputs));
            }
            raw.insert("fields".to_string(), serde_json::json!({"LIST": [data.name, list_id]}));
        }
        Block::ProcedureDef(data) => {
            raw.insert("opcode".to_string(), JsonValue::String("procedures_definition".to_string()));
            let proto_id = ctx.gen_id();
            let param_ids: Vec<Id> = (0..data.params.len()).map(|_| ctx.gen_id()).collect();

            ctx.add_func(&data.name, param_ids.clone(), data.warp);

            let mut param_block_ids = Vec::new();
            for param in &data.params {
                let param_block_id = ctx.gen_id();
                param_block_ids.push(param_block_id.clone());
                ctx.add_block(&param_block_id, &Block::RawBlock({
                    let mut r = HashMap::new();
                    r.insert("opcode".to_string(), JsonValue::String("argument_reporter_string_number".to_string()));
                    r.insert("fields".to_string(), serde_json::json!({"VALUE": [sanitize_proc_name(param, true), null]}));
                    r
                }), &BlockMeta::new(Some(proto_id.clone()), None));
            }

            let mut proto_inputs = serde_json::Map::new();
            for (param_id, block_id) in param_ids.iter().zip(param_block_ids.iter()) {
                proto_inputs.insert(param_id.clone(), serde_json::json!([1, block_id]));
            }

            let mut proto_data = HashMap::new();
            proto_data.insert("opcode".to_string(), JsonValue::String("procedures_prototype".to_string()));
            if !proto_inputs.is_empty() || !ctx.cfg.minify {
                proto_data.insert("inputs".to_string(), JsonValue::Object(proto_inputs));
            }
            let proccode = format!("{}{}", sanitize_proc_name(&data.name, false), " %s".repeat(data.params.len()));
            proto_data.insert("mutation".to_string(), serde_json::json!({
                "tagName": "mutation",
                "children": [],
                "proccode": proccode,
                "argumentids": serde_json::to_string(&param_ids).unwrap_or_default(),
                "argumentnames": serde_json::to_string(&data.params.iter().map(|p| sanitize_proc_name(p, true)).collect::<Vec<_>>()).unwrap_or_default(),
                "argumentdefaults": serde_json::to_string(&vec![""; data.params.len()]).unwrap_or_default(),
                "warp": serde_json::to_string(&data.warp).unwrap_or_default()
            }));

            ctx.add_block(&proto_id, &Block::RawBlock(proto_data), &BlockMeta::new_shadow(Some(my_id.clone()), None));
            raw.insert("inputs".to_string(), serde_json::json!({"custom_block": [1, proto_id]}));
        }
        Block::ProcedureCall(_) => {
            raw.insert("opcode".to_string(), JsonValue::String("procedures_call".to_string()));
        }
        Block::Pen(op) => {
            ctx.uses_pen = true;
            match op {
                PenOp::Down => {
                    raw.insert("opcode".to_string(), JsonValue::String("pen_penDown".to_string()));
                }
                PenOp::Up => {
                    raw.insert("opcode".to_string(), JsonValue::String("pen_penUp".to_string()));
                }
                PenOp::Clear => {
                    raw.insert("opcode".to_string(), JsonValue::String("pen_clear".to_string()));
                }
                PenOp::SetColor { color } => {
                    raw.insert("opcode".to_string(), JsonValue::String("pen_setPenColorToColor".to_string()));
                    let input = get_raw_value(color, my_id, ctx, ScratchCast::ToNum);
                    raw.insert("inputs".to_string(), serde_json::json!({"COLOR": input}));
                }
                PenOp::SetSize { size } => {
                    raw.insert("opcode".to_string(), JsonValue::String("pen_setPenSizeTo".to_string()));
                    let input = get_raw_value(size, my_id, ctx, ScratchCast::ToNum);
                    raw.insert("inputs".to_string(), serde_json::json!({"SIZE": input}));
                }
            }
        }
        Block::MotionGoto { x, y } => {
            raw.insert("opcode".to_string(), JsonValue::String("motion_gotoxy".to_string()));
            let x_input = get_raw_value(x, my_id, ctx, ScratchCast::ToNum);
            let y_input = get_raw_value(y, my_id, ctx, ScratchCast::ToNum);
            raw.insert("inputs".to_string(), serde_json::json!({"X": x_input, "Y": y_input}));
        }
        Block::RawBlock(_) => unreachable!(),
    }
    raw
}

fn get_raw_value(
    value: &Value,
    parent: &Id,
    ctx: &mut ScratchContext,
    cast: ScratchCast,
) -> Vec<JsonValue> {
    match value {
        Value::Known(kv) => {
            let (type_id, raw) = match kv {
                KnownVal::Str(s) => (10, JsonValue::String(s.clone())),
                KnownVal::Num(n) => {
                    if n.is_nan() {
                        (4, JsonValue::String("NaN".to_string()))
                    } else if n.is_infinite() {
                        (4, JsonValue::String(if *n > 0.0 { "Infinity".to_string() } else { "-Infinity".to_string() }))
                    } else if n.fract() == 0.0 {
                        let int_val = *n as i64;
                        if ctx.cfg.use_hex_if_smaller && cast == ScratchCast::ToNum && int_val > 0 {
                            let base10_digits = (int_val as f64).log10().ceil() as usize;
                            let hex_digits = 2 + ((int_val as f64 + 1.0).log2() / 4.0).ceil() as usize;
                            if hex_digits < base10_digits {
                                (4, JsonValue::String(format!("0x{:x}", int_val)))
                            } else {
                                (4, JsonValue::Number(int_val.into()))
                            }
                        } else {
                            (4, JsonValue::Number(int_val.into()))
                        }
                    } else {
                        (4, serde_json::json!(*n))
                    }
                }
                KnownVal::Bool(b) => (4, JsonValue::Number(if *b { 1 } else { 0 }.into())),
            };
            vec![JsonValue::Number(1.into()), serde_json::json!([type_id, raw])]
        }
        Value::KnownBool(b) => {
            let n = if *b { 1 } else { 0 };
            vec![JsonValue::Number(1.into()), serde_json::json!([4, n])]
        }
        Value::GetVar { name } => {
            let var_id = ctx.add_or_get_var(name, None);
            vec![JsonValue::Number(3.into()), serde_json::json!([12, name, var_id])]
        }
        Value::GetList { name } => {
            let list_id = ctx.add_or_get_list(name, vec![]);
            vec![JsonValue::Number(3.into()), serde_json::json!([13, name, list_id])]
        }
        Value::GetOfList(data) => {
            let id = ctx.gen_id();
            let list_id = ctx.add_or_get_list(&data.name, vec![]);
            let val_cast = match data.op {
                ListOp::AtIndex => ScratchCast::ToNum,
                _ => ScratchCast::ToStr,
            };
            let raw_value = get_raw_value(&data.value, parent, ctx, val_cast);
            let input_name = match data.op {
                ListOp::AtIndex => "INDEX",
                ListOp::IndexOf => "ITEM",
                ListOp::LengthOf => "INDEX",
                ListOp::Contains => "ITEM",
            };
            let opcode = opcode_from_short_op(match data.op {
                ListOp::AtIndex => "atindex",
                ListOp::IndexOf => "indexof",
                ListOp::LengthOf => "atindex",
                ListOp::Contains => "contains",
            }).unwrap_or("data_itemoflist");
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
                r.insert("inputs".to_string(), serde_json::json!({input_name: raw_value}));
                r.insert("fields".to_string(), serde_json::json!({"LIST": [data.name, list_id]}));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
        Value::GetListLength { name } => {
            let id = ctx.gen_id();
            let list_id = ctx.add_or_get_list(name, vec![]);
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String("data_lengthoflist".to_string()));
                r.insert("fields".to_string(), serde_json::json!({"LIST": [name, list_id]}));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
        Value::GetParam { name } => {
            let id = ctx.gen_id();
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String("argument_reporter_string_number".to_string()));
                r.insert("fields".to_string(), serde_json::json!({"VALUE": [sanitize_proc_name(name, true), null]}));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
        Value::Op(op) => {
            match op {
                Op::BoolToFloat(inner) | Op::StrToFloat(inner) => {
                    if cast == ScratchCast::ToNum {
                        return get_raw_value(inner, parent, ctx, cast);
                    }
                }
                _ => {}
            }

            let id = ctx.gen_id();
            let (short_op, left, right) = match op {
                Op::Add(l, r) => ("add", l.as_ref(), Some(r.as_ref())),
                Op::Sub(l, r) => ("sub", l.as_ref(), Some(r.as_ref())),
                Op::Mul(l, r) => ("mul", l.as_ref(), Some(r.as_ref())),
                Op::Div(l, r) => ("div", l.as_ref(), Some(r.as_ref())),
                Op::Mod(l, r) => ("mod", l.as_ref(), Some(r.as_ref())),
                Op::Rand(l, r) => ("rand", l.as_ref(), Some(r.as_ref())),
                Op::Join(l, r) => ("join", l.as_ref(), Some(r.as_ref())),
                Op::LetterOf(l, r) => ("letter_of", l.as_ref(), Some(r.as_ref())),
                Op::LengthOf(l) => ("length_of", l.as_ref(), None),
                Op::Round(l) => ("round", l.as_ref(), None),
                Op::Not(l) => ("not", l.as_ref(), None),
                Op::Contains(l, r) => ("contains", l.as_ref(), Some(r.as_ref())),
                Op::BoolToFloat(l) => ("bool_to_float", l.as_ref(), None),
                Op::StrToFloat(l) => ("str_to_float", l.as_ref(), None),
                Op::Abs(l) => ("abs", l.as_ref(), None),
                Op::Floor(l) => ("floor", l.as_ref(), None),
                Op::Ceiling(l) => ("ceiling", l.as_ref(), None),
                Op::Sqrt(l) => ("sqrt", l.as_ref(), None),
                Op::Sin(l) => ("sin", l.as_ref(), None),
                Op::Cos(l) => ("cos", l.as_ref(), None),
                Op::Tan(l) => ("tan", l.as_ref(), None),
                Op::Asin(l) => ("asin", l.as_ref(), None),
                Op::Acos(l) => ("acos", l.as_ref(), None),
                Op::Atan(l) => ("atan", l.as_ref(), None),
                Op::Ln(l) => ("ln", l.as_ref(), None),
                Op::Log(l) => ("log", l.as_ref(), None),
                Op::Exp(l) => ("e ^", l.as_ref(), None),
                Op::Exp10(l) => ("10 ^", l.as_ref(), None),
            };

            let opcode = opcode_from_short_op(short_op).unwrap_or("operator_mathop");

            let (lft_param, rgt_param) = match short_op {
                "rand" => ("FROM", Some("TO")),
                "join" => ("STRING1", Some("STRING2")),
                "letter_of" => ("LETTER", Some("STRING")),
                "length_of" => ("STRING", None),
                "round" | "bool_to_float" | "not" => {
                    if short_op == "not" { ("OPERAND", None) } else { ("NUM", None) }
                }
                "str_to_float" => ("NUM1", Some("NUM2")),
                _ => if right.is_some() { ("NUM1", Some("NUM2")) } else { ("NUM", None) },
            };

            let casts_left = if short_op == "join" || short_op == "length_of" {
                ScratchCast::ToStr
            } else {
                ScratchCast::ToNum
            };

            let raw_left = get_raw_value(left, &id, ctx, casts_left);
            let mut inputs = serde_json::Map::new();
            inputs.insert(lft_param.to_string(), serde_json::json!(raw_left));

            let effective_right = if short_op == "str_to_float" {
                &Value::Known(KnownVal::Num(0.0))
            } else if let Some(r) = right {
                r
            } else {
                return finalize_op_block(ctx, id, parent, opcode, inputs, short_op, false);
            };

            let casts_right = if short_op == "letter_of" { ScratchCast::ToStr } else { casts_left };
            let raw_right = get_raw_value(effective_right, &id, ctx, casts_right);
            if let Some(rp) = rgt_param {
                inputs.insert(rp.to_string(), serde_json::json!(raw_right));
            }

            finalize_op_block(ctx, id, parent, opcode, inputs, short_op, false)
        }
        Value::BoolOp(bop) => {
            let id = ctx.gen_id();
            let (short_op, left, right) = match bop {
                BoolOp::And(l, r) => ("and", l.as_ref(), Some(r.as_ref())),
                BoolOp::Or(l, r) => ("or", l.as_ref(), Some(r.as_ref())),
                BoolOp::Eq(l, r) => ("=", l.as_ref(), Some(r.as_ref())),
                BoolOp::Lt(l, r) => ("<", l.as_ref(), Some(r.as_ref())),
                BoolOp::Gt(l, r) => (">", l.as_ref(), Some(r.as_ref())),
                BoolOp::Not(l) => ("not", l.as_ref(), None),
            };

            let opcode = opcode_from_short_op(short_op).unwrap_or("operator_not");

            let (lft_param, rgt_param) = match short_op {
                "not" => ("OPERAND", None),
                "contains" => ("STRING1", Some("STRING2")),
                _ => ("OPERAND1", Some("OPERAND2")),
            };

            let mut inputs = serde_json::Map::new();

            if short_op == "not" || short_op == "and" || short_op == "or" {
                if let Some(raw_left) = get_raw_bool_value(left, &id, ctx) {
                    inputs.insert(lft_param.to_string(), raw_left);
                }
                if let Some(r) = right
                    && let Some(raw_right) = get_raw_bool_value(r, &id, ctx)
                        && let Some(rp) = rgt_param {
                            inputs.insert(rp.to_string(), raw_right);
                        }
            } else {
                let raw_left = get_raw_value(left, &id, ctx, ScratchCast::ToStr);
                inputs.insert(lft_param.to_string(), serde_json::json!(raw_left));
                if let Some(r) = right {
                    let raw_right = get_raw_value(r, &id, ctx, ScratchCast::ToStr);
                    if let Some(rp) = rgt_param {
                        inputs.insert(rp.to_string(), serde_json::json!(raw_right));
                    }
                }
            }

            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
                if !inputs.is_empty() {
                    r.insert("inputs".to_string(), JsonValue::Object(inputs));
                }
                r
            }), &BlockMeta::new(Some(parent.clone()), None));

            vec![JsonValue::Number(2.into()), JsonValue::String(id)]
        }
        Value::CostumeInfo { op } => {
            let id = ctx.gen_id();
            let op_str = match op {
                CostumeInfoOp::Name => "name",
                CostumeInfoOp::Number => "number",
            };
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String("looks_costumenumbername".to_string()));
                r.insert("fields".to_string(), serde_json::json!({"NUMBER_NAME": [op_str, null]}));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
        Value::GetCounter => {
            if !ctx.cfg.allow_hacked_blocks {
                return get_raw_value(&Value::GetVar { name: COUNTER_REPLACEMENT_NAME.to_string() }, parent, ctx, cast);
            }
            let id = ctx.gen_id();
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String("control_get_counter".to_string()));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
        Value::GetAnswer => {
            let id = ctx.gen_id();
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String("sensing_answer".to_string()));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
        Value::DaysSince2000 => {
            let id = ctx.gen_id();
            ctx.add_block(&id, &Block::RawBlock({
                let mut r = HashMap::new();
                r.insert("opcode".to_string(), JsonValue::String("sensing_dayssince2000".to_string()));
                r
            }), &BlockMeta::new(Some(parent.clone()), None));
            vec![JsonValue::Number(3.into()), JsonValue::String(id)]
        }
    }
}

fn finalize_op_block(
    ctx: &mut ScratchContext,
    id: Id,
    parent: &Id,
    opcode: &str,
    inputs: serde_json::Map<String, JsonValue>,
    short_op: &str,
    _is_bool: bool,
) -> Vec<JsonValue> {
    let mut fields = serde_json::Map::new();
    if opcode == "operator_mathop" {
        fields.insert("OPERATOR".to_string(), serde_json::json!([short_op, null]));
    }

    ctx.add_block(&id, &Block::RawBlock({
        let mut r = HashMap::new();
        r.insert("opcode".to_string(), JsonValue::String(opcode.to_string()));
        if !inputs.is_empty() {
            r.insert("inputs".to_string(), JsonValue::Object(inputs));
        }
        if !fields.is_empty() {
            r.insert("fields".to_string(), JsonValue::Object(fields));
        }
        r
    }), &BlockMeta::new(Some(parent.clone()), None));

    vec![JsonValue::Number(3.into()), JsonValue::String(id)]
}

fn get_raw_bool_value(
    value: &Value,
    parent: &Id,
    ctx: &mut ScratchContext,
) -> Option<JsonValue> {
    match value {
        Value::BoolOp(_bop) => {
            let raw = get_raw_value(value, parent, ctx, ScratchCast::ToNum);
            Some(serde_json::json!(raw))
        }
        Value::KnownBool(false) => None,
        Value::KnownBool(true) => {
            get_raw_bool_value(&Value::BoolOp(BoolOp::Not(Box::new(Value::KnownBool(false)))), parent, ctx)
        }
        Value::Known(kv) => {
            let b = scratch_cast_to_bool(kv);
            if b {
                get_raw_bool_value(&Value::KnownBool(true), parent, ctx)
            } else {
                None
            }
        }
        _ => {
            let raw = get_raw_value(value, parent, ctx, ScratchCast::ToNum);
            Some(serde_json::json!(raw))
        }
    }
}

pub fn sanitize_proc_name(name: &str, is_param: bool) -> String {
    if is_param && (name == "%b" || name == "%n") {
        return name.replace('%', "\u{FF05}");
    }
    if !is_param && name == "%" {
        return name.replace('%', "\u{FF05}");
    }
    if !is_param && name == "hasOwnProperty" {
        return format!("{}:bro why", name);
    }
    name.to_string()
}

pub fn scratch_cast_to_str(val: &KnownVal) -> String {
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
            } else if n.fract() == 0.0 {
                (*n as i64).to_string()
            } else {
                n.to_string()
            }
        }
    }
}

pub fn scratch_cast_to_num(val: &KnownVal) -> f64 {
    match val {
        KnownVal::Str(s) => s.parse::<f64>().unwrap_or(0.0),
        KnownVal::Num(n) => *n,
        KnownVal::Bool(b) => if *b { 1.0 } else { 0.0 },
    }
}

pub fn scratch_cast_to_bool(val: &KnownVal) -> bool {
    match val {
        KnownVal::Str(s) => {
            let lower = s.to_lowercase();
            !lower.is_empty() && lower != "0" && lower != "false"
        }
        KnownVal::Num(n) => *n != 0.0 && !n.is_nan(),
        KnownVal::Bool(b) => *b,
    }
}

pub fn scratch_compare(left: &KnownVal, right: &KnownVal) -> f64 {
    let left_num = match left {
        KnownVal::Str(s) => s.parse::<f64>().ok(),
        KnownVal::Num(n) => Some(*n),
        KnownVal::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
    };
    let right_num = match right {
        KnownVal::Str(s) => s.parse::<f64>().ok(),
        KnownVal::Num(n) => Some(*n),
        KnownVal::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
    };

    match (left_num, right_num) {
        (Some(l), Some(r)) => {
            if l.is_infinite() && r.is_infinite()
                && ((l.is_sign_positive() && r.is_sign_positive())
                    || (l.is_sign_negative() && r.is_sign_negative()))
            {
                0.0
            } else {
                l - r
            }
        }
        _ => {
            let l_str = scratch_cast_to_str(left).to_lowercase();
            let r_str = scratch_cast_to_str(right).to_lowercase();
            if l_str == r_str { 0.0 } else if l_str < r_str { -1.0 } else { 1.0 }
        }
    }
}

pub fn make_empty_costume(name: &str) -> HashMap<String, JsonValue> {
    let mut costume = HashMap::new();
    let hash = empty_svg_hash();
    costume.insert("name".to_string(), JsonValue::String(name.to_string()));
    costume.insert("bitmapResolution".to_string(), JsonValue::Number(1.into()));
    costume.insert("dataFormat".to_string(), JsonValue::String("svg".to_string()));
    costume.insert("assetId".to_string(), JsonValue::String(hash.clone()));
    costume.insert("md5ext".to_string(), JsonValue::String(format!("{}.svg", hash)));
    costume.insert("rotationCenterX".to_string(), JsonValue::Number(0.into()));
    costume.insert("rotationCenterY".to_string(), JsonValue::Number(0.into()));
    costume
}

pub fn empty_svg_hash() -> String {
    use md5::Digest;
    let mut hasher = md5::Md5::new();
    hasher.update(EMPTY_SVG.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_new() {
        let project = Project::new(ScratchConfig::default());
        assert!(project.code.is_empty());
        assert!(project.lists.is_empty());
    }

    #[test]
    fn test_scratch_context_gen_id() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let id1 = ctx.gen_id();
        let id2 = ctx.gen_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_scratch_context_add_var() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let id = ctx.add_or_get_var("x", None);
        assert!(!id.is_empty());
        let id2 = ctx.add_or_get_var("x", None);
        assert_eq!(id, id2);
    }

    #[test]
    fn test_scratch_context_add_list() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let id = ctx.add_or_get_list("mylist", vec![]);
        assert!(!id.is_empty());
    }

    #[test]
    fn test_scratch_context_add_broadcast() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let id1 = ctx.add_broadcast("msg1");
        let id2 = ctx.add_broadcast("msg1");
        assert_eq!(id1, id2);
        let id3 = ctx.add_broadcast("msg2");
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_empty_svg_hash() {
        let hash = empty_svg_hash();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_make_empty_costume() {
        let costume = make_empty_costume("test");
        assert_eq!(costume["name"], JsonValue::String("test".to_string()));
        assert!(costume.contains_key("assetId"));
        assert!(costume.contains_key("md5ext"));
        assert!(costume.contains_key("dataFormat"));
        assert_eq!(costume["dataFormat"], JsonValue::String("svg".to_string()));
    }

    #[test]
    fn test_scratch_cast_to_str() {
        assert_eq!(scratch_cast_to_str(&KnownVal::Num(42.0)), "42");
        assert_eq!(scratch_cast_to_str(&KnownVal::Num(3.14)), "3.14");
        assert_eq!(scratch_cast_to_str(&KnownVal::Str("hello".to_string())), "hello");
        assert_eq!(scratch_cast_to_str(&KnownVal::Bool(true)), "true");
        assert_eq!(scratch_cast_to_str(&KnownVal::Bool(false)), "false");
        assert_eq!(scratch_cast_to_str(&KnownVal::Num(f64::INFINITY)), "Infinity");
        assert_eq!(scratch_cast_to_str(&KnownVal::Num(f64::NEG_INFINITY)), "-Infinity");
    }

    #[test]
    fn test_scratch_cast_to_num() {
        assert_eq!(scratch_cast_to_num(&KnownVal::Str("42".to_string())), 42.0);
        assert_eq!(scratch_cast_to_num(&KnownVal::Str("abc".to_string())), 0.0);
        assert_eq!(scratch_cast_to_num(&KnownVal::Num(3.14)), 3.14);
        assert_eq!(scratch_cast_to_num(&KnownVal::Bool(true)), 1.0);
        assert_eq!(scratch_cast_to_num(&KnownVal::Bool(false)), 0.0);
    }

    #[test]
    fn test_scratch_cast_to_bool() {
        assert!(scratch_cast_to_bool(&KnownVal::Num(1.0)));
        assert!(!scratch_cast_to_bool(&KnownVal::Num(0.0)));
        assert!(scratch_cast_to_bool(&KnownVal::Str("hello".to_string())));
        assert!(!scratch_cast_to_bool(&KnownVal::Str("".to_string())));
        assert!(!scratch_cast_to_bool(&KnownVal::Str("false".to_string())));
        assert!(!scratch_cast_to_bool(&KnownVal::Str("0".to_string())));
        assert!(scratch_cast_to_bool(&KnownVal::Bool(true)));
        assert!(!scratch_cast_to_bool(&KnownVal::Bool(false)));
    }

    #[test]
    fn test_scratch_compare() {
        assert_eq!(scratch_compare(&KnownVal::Num(5.0), &KnownVal::Num(3.0)), 2.0);
        assert_eq!(scratch_compare(&KnownVal::Str("a".to_string()), &KnownVal::Str("b".to_string())), -1.0);
        assert_eq!(scratch_compare(&KnownVal::Str("b".to_string()), &KnownVal::Str("a".to_string())), 1.0);
        assert_eq!(scratch_compare(&KnownVal::Str("hello".to_string()), &KnownVal::Str("hello".to_string())), 0.0);
        assert_eq!(scratch_compare(&KnownVal::Num(f64::INFINITY), &KnownVal::Num(f64::INFINITY)), 0.0);
        assert_eq!(scratch_compare(&KnownVal::Num(f64::NEG_INFINITY), &KnownVal::Num(f64::NEG_INFINITY)), 0.0);
    }

    #[test]
    fn test_sanitize_proc_name() {
        assert_eq!(sanitize_proc_name("myFunc", false), "myFunc");
        assert_eq!(sanitize_proc_name("%b", true), "\u{FF05}b");
        assert_eq!(sanitize_proc_name("%n", true), "\u{FF05}n");
        assert_eq!(sanitize_proc_name("%", false), "\u{FF05}");
        assert_eq!(sanitize_proc_name("hasOwnProperty", false), "hasOwnProperty:bro why");
    }

    #[test]
    fn test_get_raw_value_known_num() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::Known(KnownVal::Num(42.0)), &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], JsonValue::Number(1.into()));
    }

    #[test]
    fn test_get_raw_value_known_str() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::Known(KnownVal::Str("hello".to_string())), &parent, &mut ctx, ScratchCast::ToStr);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], JsonValue::Number(1.into()));
    }

    #[test]
    fn test_get_raw_value_known_bool() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::KnownBool(true), &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], JsonValue::Number(1.into()));
    }

    #[test]
    fn test_get_raw_value_get_var() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::GetVar { name: "x".to_string() }, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], JsonValue::Number(3.into()));
        assert!(ctx.vars.contains_key("x"));
    }

    #[test]
    fn test_get_raw_value_get_list() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::GetList { name: "mylist".to_string() }, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], JsonValue::Number(3.into()));
        assert!(ctx.lists.contains_key("mylist"));
    }

    #[test]
    fn test_get_raw_block_say() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::Say { value: Value::Known(KnownVal::Str("Hello!".to_string())) };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("looks_say".to_string()));
        assert!(raw.contains_key("inputs"));
    }

    #[test]
    fn test_get_raw_block_on_start_flag() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let raw = get_raw_block(&Block::OnStartFlag, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("event_whenflagclicked".to_string()));
    }

    #[test]
    fn test_get_raw_block_stop_script() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let raw = get_raw_block(&Block::StopScript(StopOption::All), &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("control_stop".to_string()));
        assert!(raw.contains_key("fields"));
    }

    #[test]
    fn test_get_raw_block_edit_var_set() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "x".to_string(),
            value: Value::Known(KnownVal::Num(10.0)),
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("data_setvariableto".to_string()));
        assert!(raw.contains_key("inputs"));
        assert!(raw.contains_key("fields"));
        assert!(ctx.vars.contains_key("x"));
    }

    #[test]
    fn test_get_raw_block_edit_var_change() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::EditVar(EditVarData {
            op: VarOp::Change,
            name: "counter".to_string(),
            value: Value::Known(KnownVal::Num(1.0)),
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("data_changevariableby".to_string()));
    }

    #[test]
    fn test_get_raw_block_wait() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::Wait { value: Value::Known(KnownVal::Num(1.0)) };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("control_wait".to_string()));
        assert!(raw.contains_key("inputs"));
    }

    #[test]
    fn test_get_raw_block_broadcast() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::Broadcast {
            value: Value::Known(KnownVal::Str("event1".to_string())),
            wait: false,
        };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("event_broadcast".to_string()));
        assert!(raw.contains_key("inputs"));
    }

    #[test]
    fn test_get_raw_block_broadcast_and_wait() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::Broadcast {
            value: Value::Known(KnownVal::Str("event1".to_string())),
            wait: true,
        };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("event_broadcastandwait".to_string()));
    }

    #[test]
    fn test_get_raw_block_on_broadcast() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::OnBroadcast { name: "msg".to_string() };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("event_whenbroadcastreceived".to_string()));
        assert!(raw.contains_key("fields"));
    }

    #[test]
    fn test_get_raw_block_control_if() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::ControlFlow(ControlFlow {
            op: ControlOp::If,
            condition: Some(Value::KnownBool(true)),
            body: Some(BlockList::from_blocks(vec![Block::Say { value: Value::Known(KnownVal::Str("yes".to_string())) }])),
            else_body: None,
            var: None,
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("control_if".to_string()));
        assert!(raw.contains_key("inputs"));
    }

    #[test]
    fn test_get_raw_block_control_if_else() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::ControlFlow(ControlFlow {
            op: ControlOp::IfElse,
            condition: Some(Value::KnownBool(true)),
            body: Some(BlockList::from_blocks(vec![])),
            else_body: Some(BlockList::from_blocks(vec![])),
            var: None,
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("control_if_else".to_string()));
    }

    #[test]
    fn test_get_raw_block_control_forever() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::ControlFlow(ControlFlow {
            op: ControlOp::Forever,
            condition: None,
            body: Some(BlockList::from_blocks(vec![])),
            else_body: None,
            var: None,
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("control_forever".to_string()));
    }

    #[test]
    fn test_get_raw_block_switch_costume() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::SwitchCostume { value: Value::Known(KnownVal::Str("costume1".to_string())) };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("looks_switchcostumeto".to_string()));
    }

    #[test]
    fn test_get_raw_block_edit_volume() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::EditVolume { op: VolumeOp::Set, value: Value::Known(KnownVal::Num(100.0)) };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("sound_setvolumeto".to_string()));
    }

    #[test]
    fn test_get_raw_block_edit_counter_no_hacked() {
        let mut ctx = ScratchContext::new(ScratchConfig { allow_hacked_blocks: false, ..ScratchConfig::default() });
        let my_id = ctx.gen_id();
        let block = Block::EditCounter(CounterOp::Increment);
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("data_changevariableby".to_string()));
    }

    #[test]
    fn test_get_raw_block_edit_counter_hacked() {
        let mut ctx = ScratchContext::new(ScratchConfig { allow_hacked_blocks: true, ..ScratchConfig::default() });
        let my_id = ctx.gen_id();
        let block = Block::EditCounter(CounterOp::Increment);
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("control_incr_counter".to_string()));
    }

    #[test]
    fn test_get_raw_block_ask() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::Ask { value: Value::Known(KnownVal::Str("What?".to_string())), var_name: None };
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("sensing_askandwait".to_string()));
    }

    #[test]
    fn test_get_raw_block_procedure_def() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let my_id = ctx.gen_id();
        let block = Block::ProcedureDef(ProcedureDefData {
            name: "myFunc".to_string(),
            params: vec!["x".to_string(), "y".to_string()],
            warp: false,
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        assert_eq!(raw["opcode"], JsonValue::String("procedures_definition".to_string()));
        assert!(raw.contains_key("inputs"));
        assert!(ctx.funcs.contains_key("myFunc"));
    }

    #[test]
    fn test_add_block_list() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let bl = BlockList::from_blocks(vec![
            Block::OnStartFlag,
            Block::Say { value: Value::Known(KnownVal::Str("Hello!".to_string())) },
        ]);
        let id = ctx.add_block_list(&bl, None);
        assert!(id.is_some());
        assert!(ctx.blocks.len() >= 2);
    }

    #[test]
    fn test_project_get_ctx() {
        let cfg = ScratchConfig::default();
        let project = Project::new(cfg.clone());
        let ctx = project.get_ctx();
        assert!(ctx.blocks.is_empty());
        assert_eq!(ctx.cfg, cfg);
    }

    #[test]
    fn test_scratch_config_minify() {
        let cfg = ScratchConfig { minify: true, ..ScratchConfig::default() };
        let mut ctx = ScratchContext::new(cfg);
        let my_id = ctx.gen_id();
        let block = Block::EditVar(EditVarData {
            op: VarOp::Set,
            name: "x".to_string(),
            value: Value::Known(KnownVal::Num(5.0)),
        });
        let raw = get_raw_block(&block, &my_id, &mut ctx);
        let fields = raw["fields"].as_object().unwrap();
        let var_field = fields["VARIABLE"].as_array().unwrap();
        // Names are preserved to match the Python implementation's output.
        assert_eq!(var_field[0], JsonValue::String("x".to_string()));
    }

    #[test]
    fn test_get_raw_value_op_add() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let val = Value::Op(Op::Add(
            Box::new(Value::Known(KnownVal::Num(3.0))),
            Box::new(Value::Known(KnownVal::Num(4.0))),
        ));
        let result = get_raw_value(&val, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result[0], JsonValue::Number(3.into()));
        assert!(ctx.blocks.len() >= 1);
    }

    #[test]
    fn test_get_raw_value_bool_op_eq() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let val = Value::BoolOp(BoolOp::Eq(
            Box::new(Value::Known(KnownVal::Num(3.0))),
            Box::new(Value::Known(KnownVal::Num(3.0))),
        ));
        let result = get_raw_value(&val, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result[0], JsonValue::Number(2.into()));
    }

    #[test]
    fn test_get_raw_value_costume_info() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::CostumeInfo { op: CostumeInfoOp::Name }, &parent, &mut ctx, ScratchCast::ToStr);
        assert_eq!(result[0], JsonValue::Number(3.into()));
    }

    #[test]
    fn test_get_raw_value_get_answer() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::GetAnswer, &parent, &mut ctx, ScratchCast::ToStr);
        assert_eq!(result[0], JsonValue::Number(3.into()));
    }

    #[test]
    fn test_get_raw_value_days_since_2000() {
        let mut ctx = ScratchContext::new(ScratchConfig::default());
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::DaysSince2000, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result[0], JsonValue::Number(3.into()));
    }

    #[test]
    fn test_get_raw_value_get_counter_no_hacked() {
        let mut ctx = ScratchContext::new(ScratchConfig { allow_hacked_blocks: false, ..ScratchConfig::default() });
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::GetCounter, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result[0], JsonValue::Number(3.into()));
        assert!(ctx.vars.contains_key(COUNTER_REPLACEMENT_NAME));
    }

    #[test]
    fn test_get_raw_value_get_counter_hacked() {
        let mut ctx = ScratchContext::new(ScratchConfig { allow_hacked_blocks: true, ..ScratchConfig::default() });
        let parent = ctx.gen_id();
        let result = get_raw_value(&Value::GetCounter, &parent, &mut ctx, ScratchCast::ToNum);
        assert_eq!(result[0], JsonValue::Number(3.into()));
    }
}