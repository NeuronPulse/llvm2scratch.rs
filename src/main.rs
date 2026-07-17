use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::process;

use clap::builder::PossibleValuesParser;

use llvm2scratch::compiler::config::{CompilerConfig, BINOP_LOOKUP_BITS, VARIABLE_MAX_BITS};
use llvm2scratch::compiler::translate;
use llvm2scratch::target::BranchMethod;
use llvm2scratch::optimizer::Optimization;
use llvm2scratch::scratch::ast::{Format, KnownVal, ScratchConfig, Value};
use llvm2scratch::scratch::export::export_scratch_file;
use llvm2scratch::scratch::{Project, ScratchContext};
use llvm2scratch::target::loader::get_target;
use llvm2scratch::target::{DEFAULT_OPT_TARGET, DEFAULT_TARGETS};

fn project_to_context(proj: &Project) -> ScratchContext {
    let mut ctx = ScratchContext::new(proj.cfg.clone());

    for code in &proj.code {
        ctx.add_block_list(code, None);
    }

    for (name, vals) in &proj.lists {
        ctx.add_or_get_list(name, vals.clone());
    }

    ctx
}

fn parse_optimization_options(values: Option<clap::parser::ValuesRef<'_, String>>) -> (bool, HashSet<Optimization>) {
    let opts: Vec<String> = values.map(|v| v.cloned().collect()).unwrap_or_default();
    if opts.is_empty() || opts.contains(&"all".to_string()) {
        let mut passes = HashSet::new();
        passes.insert(Optimization::KnownValuePropagation);
        passes.insert(Optimization::AssignmentElision);
        return (true, passes);
    }
    if opts.contains(&"none".to_string()) {
        return (false, HashSet::new());
    }
    let mut compiler_opt = opts.contains(&"compiler".to_string());
    let mut passes = HashSet::new();
    for o in opts {
        match o.as_str() {
            "compiler" => compiler_opt = true,
            "known_value_propagation" => {
                passes.insert(Optimization::KnownValuePropagation);
            }
            "assignment_elision" => {
                passes.insert(Optimization::AssignmentElision);
            }
            _ => {}
        }
    }
    (compiler_opt, passes)
}

fn parse_minify_options(values: Option<clap::parser::ValuesRef<'_, String>>) -> (bool, bool, bool, bool) {
    let opts: Vec<String> = values.map(|v| v.cloned().collect()).unwrap_or_default();
    if opts.is_empty() {
        return (true, false, false, false);
    }
    if opts.contains(&"all".to_string()) {
        return (true, true, true, true);
    }
    if opts.contains(&"none".to_string()) {
        return (false, false, false, false);
    }
    let minify = opts.contains(&"general".to_string());
    let compiler_minify = opts.contains(&"compiler".to_string());
    let break_glow = opts.contains(&"break-glow".to_string());
    let gen_lut_runtime = opts.contains(&"gen-lut-runtime".to_string());
    (minify, compiler_minify, break_glow, gen_lut_runtime)
}

fn infer_format(output: &str, explicit: Option<&String>) -> Result<Format, String> {
    if let Some(f) = explicit {
        return match f.as_str() {
            "project3" => Ok(Format::Project3),
            "sprite3" => Ok(Format::Sprite3),
            _ => Err(format!("Unknown format: {}", f)),
        };
    }
    let ext = std::path::Path::new(output)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext {
        "sb3" => Ok(Format::Project3),
        "sprite3" => Ok(Format::Sprite3),
        _ => Err(format!(
            "Could not infer output file format from extension \"{}\". Either use a valid extension or set -f/--format",
            ext
        )),
    }
}

fn main() {
    let matches = clap::Command::new("llvm2scratch")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Compile LLVM 19 IR (.ll) files into Scratch projects or sprites")
        .arg(
            clap::Arg::new("input")
                .help("Path to the input LLVM 19 IR (.ll) file")
                .required(true)
                .index(1),
        )
        .arg(
            clap::Arg::new("output_pos")
                .help("Path to the output file (.sb3 or .sprite3)")
                .required(false)
                .index(2),
        )
        .arg(
            clap::Arg::new("output")
                .short('o')
                .long("output")
                .value_name("OUTPUT")
                .help("Path to the output file (.sb3 or .sprite3)"),
        )
        .arg(
            clap::Arg::new("format")
                .short('f')
                .long("format")
                .value_name("FORMAT")
                .help("File format of output file. By default this is inferred by the output file's extension.")
                .value_parser(PossibleValuesParser::new(["infer", "project3", "sprite3"]))
                .default_value("infer"),
        )
        .arg(
            clap::Arg::new("targets")
                .short('T')
                .long("targets")
                .value_name("TARGET")
                .help("Compile code to support these targets. Defaults to scratch3 turbowarp3.")
                .num_args(1..)
                .default_values(DEFAULT_TARGETS.iter().copied()),
        )
        .arg(
            clap::Arg::new("opt-target")
                .short('U')
                .long("opt-target")
                .value_name("TARGET")
                .help("Optimize code with this target in mind. Defaults to turbowarp3 if available, otherwise the first target listed."),
        )
        .arg(
            clap::Arg::new("optimizations")
                .short('O')
                .long("optimizations")
                .value_name("OPT_OPTIONS")
                .help("Optimizations to apply; defaults to all")
                .num_args(0..)
                .value_parser(PossibleValuesParser::new([
                    "all", "none", "compiler", "known_value_propagation", "assignment_elision",
                ])),
        )
        .arg(
            clap::Arg::new("minify")
                .short('M')
                .long("minify")
                .value_name("MINIFY_OPTIONS")
                .help("Minify settings to apply; defaults to general")
                .num_args(0..)
                .value_parser(PossibleValuesParser::new([
                    "all", "none", "general", "compiler", "break-glow", "gen-lut-runtime",
                ])),
        )
        .arg(
            clap::Arg::new("memory-size")
                .long("memory-size")
                .short('m')
                .value_name("MEMORY_SIZE")
                .help("Number of 'bytes' on the 'memory' list; max value is 200,000; default is 4096")
                .default_value("4096"),
        )
        .arg(
            clap::Arg::new("local-stack-size")
                .long("local-stack-size")
                .value_name("LOCAL_STACK_SIZE")
                .help("Number of 'bytes' on the local stack list for storing registers when recursing; max value is 200,000; default is 512")
                .default_value("512"),
        )
        .arg(
            clap::Arg::new("max-branch-recursion")
                .long("max-branch-recursion")
                .value_name("MAX_BRANCH_RECURSION")
                .help("Maximum depth of Scratch's call stack before resetting it; default depends on targets enabled"),
        )
        .arg(
            clap::Arg::new("no-accurate-byte-spacing")
                .long("no-accurate-byte-spacing")
                .action(clap::ArgAction::SetTrue)
                .help("Disable extra padding bytes added to each value in memory"),
        )
        .arg(
            clap::Arg::new("entrypoint")
                .long("entrypoint")
                .value_name("ENTRYPOINT")
                .help("Specify a custom entrypoint function to run once the program starts. Defaults to main.")
                .default_value("main"),
        )
        .arg(
            clap::Arg::new("debug-scratch-text")
                .long("debug-scratch-text")
                .value_name("FILE")
                .help("Output readable scratch code to a text file so it can be viewed"),
        )
        .arg(
            clap::Arg::new("debug-scratchblocks")
                .long("debug-scratchblocks")
                .value_name("FILE")
                .help("Output scratchblocks compatible code to a text file so it can be viewed. See https://scratchblocks.github.io/"),
        )
        .arg(
            clap::Arg::new("replace-hacked-blocks")
                .long("replace-hacked-blocks")
                .action(clap::ArgAction::SetTrue)
                .help("Remove 'hacked' blocks not normally accessible from the editor by replacing them with workarounds"),
        )
        .arg(
            clap::Arg::new("hide-blocks")
                .long("hide-blocks")
                .action(clap::ArgAction::SetTrue)
                .help("Prevent blocks from rendering in the editor by setting shadow: true on top level blocks"),
        )
        .arg(
            clap::Arg::new("no-optimize")
                .long("no-optimize")
                .action(clap::ArgAction::SetTrue)
                .help("Disable optimizations"),
        )
        .arg(
            clap::Arg::new("verify")
                .long("verify")
                .action(clap::ArgAction::SetTrue)
                .help("Verify LLVM IR before compiling"),
        )
        .get_matches();

    let input_path: PathBuf = matches.get_one::<String>("input").unwrap().into();

    let output_path: PathBuf = match (matches.get_one::<String>("output"), matches.get_one::<String>("output_pos")) {
        (Some(o), _) => o.into(),
        (None, Some(o)) => o.into(),
        (None, None) => {
            eprintln!("Error: no output file specified (use -o OUTPUT or a second positional argument)");
            process::exit(1);
        }
    };

    let format_arg = matches.get_one::<String>("format").map(|s| s.as_str());
    let format = match infer_format(
        output_path.to_str().unwrap_or(""),
        if format_arg == Some("infer") { None } else { matches.get_one::<String>("format") },
    ) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    };

    let entrypoint = matches.get_one::<String>("entrypoint").unwrap();
    let target_names: Vec<String> = matches
        .get_many::<String>("targets")
        .unwrap()
        .cloned()
        .collect();

    let memory_size: usize = matches
        .get_one::<String>("memory-size")
        .unwrap()
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("Invalid memory size");
            process::exit(1);
        });

    let local_stack_size: usize = matches
        .get_one::<String>("local-stack-size")
        .unwrap()
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("Invalid local stack size");
            process::exit(1);
        });

    let max_branch_recursion: Option<usize> = matches
        .get_one::<String>("max-branch-recursion")
        .map(|s| {
            s.parse().unwrap_or_else(|_| {
                eprintln!("Invalid max branch recursion");
                process::exit(1);
            })
        });

    let no_accurate_byte_spacing = matches.get_flag("no-accurate-byte-spacing");
    let replace_hacked_blocks = matches.get_flag("replace-hacked-blocks");
    let hide_blocks = matches.get_flag("hide-blocks");
    let no_optimize = matches.get_flag("no-optimize");
    let _verify = matches.get_flag("verify");

    let opt_options = matches.contains_id("optimizations").then(|| matches.get_many::<String>("optimizations").unwrap());
    let (mut compiler_opt, mut opt_passes) = parse_optimization_options(opt_options);

    let minify_options = matches.contains_id("minify").then(|| matches.get_many::<String>("minify").unwrap());
    let (minify, compiler_minify, minify_break_glow, gen_lut_runtime) = parse_minify_options(minify_options);

    let llvm_ir = match fs::read_to_string(&input_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("Error reading input file '{}': {}", input_path.display(), e);
            process::exit(1);
        }
    };

    let mut targets = Vec::new();
    for name in &target_names {
        match get_target(name) {
            Ok(t) => targets.push(t),
            Err(e) => {
                eprintln!("Unknown target '{}': {}", name, e);
                process::exit(1);
            }
        }
    }

    // Determine optimization target.
    let opt_target_name = matches.get_one::<String>("opt-target").cloned();
    let opt_target = if let Some(name) = opt_target_name {
        if !target_names.contains(&name) {
            eprintln!(
                "Optimization target (-U/--opt-target) {} should be in supported targets (-T/--targets) {}",
                name,
                target_names.join(" ")
            );
            process::exit(1);
        }
        get_target(&name).unwrap()
    } else {
        let mut chosen = targets.first().cloned().unwrap();
        for t in &targets {
            if t.id == DEFAULT_OPT_TARGET {
                chosen = t.clone();
                break;
            }
        }
        chosen
    };

    // Validate format support for each target.
    let format_value = match format {
        Format::Project3 => "project3",
        Format::Sprite3 => "sprite3",
    };
    for t in &targets {
        if !t.info.formats.contains(&format_value.to_string()) {
            eprintln!(
                "Target (-T/--targets) {} does not support format (-f/--format) {}",
                t.id, format_value
            );
            process::exit(1);
        }
    }

    // Compute default max branch recursion if not provided.
    let max_allowed_branch_recursion = targets
        .iter()
        .map(|t| t.exec.max_branch_recursion as usize)
        .min()
        .unwrap_or(0);
    let preferred_branch_recursion = opt_target.exec.preferred_branch_recursion as usize;
    let max_branch_recursion = max_branch_recursion
        .map(|v| v.min(max_allowed_branch_recursion))
        .unwrap_or_else(|| preferred_branch_recursion.min(max_allowed_branch_recursion));

    if no_optimize {
        compiler_opt = false;
        opt_passes.clear();
    }

    let mut cfg = CompilerConfig::default();
    cfg.entrypoint = entrypoint.clone();
    cfg.memory_size = memory_size;
    cfg.local_stack_size = local_stack_size;
    cfg.max_branch_recursion = max_branch_recursion;
    cfg.accurate_byte_spacing = !no_accurate_byte_spacing;
    cfg.targets = targets.clone();
    cfg.opt_target = opt_target.clone();
    cfg.compiler_opt = compiler_opt;
    cfg.compiler_minify = compiler_minify;
    cfg.opt_passes = opt_passes;
    cfg.use_branch_jump_table = opt_target.exec.preferred_branch_method == BranchMethod::JumpTable;
    cfg.gen_lut_runtime = gen_lut_runtime;
    cfg.scratch_config = ScratchConfig {
        minify,
        minify_break_glow,
        hide_blocks,
        allow_hacked_blocks: !replace_hacked_blocks,
        ..ScratchConfig::default()
    };

    let compiler_cfg = cfg.clone();
    let (mut proj, highest_return_size, functions) = match translate::compile(&llvm_ir, Some(cfg)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Compilation error: {}", e);
            process::exit(1);
        }
    };

    let optimization_enabled = !no_optimize && !compiler_cfg.opt_passes.is_empty();

    if optimization_enabled {
        let mut dont_remove = HashSet::new();
        dont_remove.insert(compiler_cfg.return_var.clone());
        dont_remove.insert(compiler_cfg.jump_table_id_var.clone());
        // Mirror Python: protect indexed return value slots up to
        // highest_return_size. Variables like "!return value:1" that are
        // beyond this range are NOT protected and can be elided if unused.
        if let Some(size) = highest_return_size {
            for i in 0..size {
                dont_remove.insert(format!("{}:{}", compiler_cfg.return_var, i));
            }
        }

        let mut ignore_external_change = HashSet::new();
        ignore_external_change.insert(compiler_cfg.stack_pointer_var.clone());

        // Build the list_lookup closure (mirrors Python's `tableLookup`).
        let mem_var = compiler_cfg.mem_var.clone();
        let and_table = format!("!AND lookup{}", compiler_cfg.zero_indexed_suffix);
        let or_table = format!("!OR lookup{}", compiler_cfg.zero_indexed_suffix);
        let xor_table = format!("!XOR lookup{}", compiler_cfg.zero_indexed_suffix);
        let pow2_table = compiler_cfg.pow2_lookup_var.clone();
        let pow2_offset = (VARIABLE_MAX_BITS + 1) as f64;
        let list_lookup = move |table_name: &str, index_val: &Value| -> Option<Value> {
            if table_name == mem_var {
                return None;
            }
            // Scratch always floors indices. `scratchCastToNum` semantics:
            // NaN -> 0.0, parse failure -> 0.0.
            let index = match index_val {
                Value::Known(KnownVal::Num(n)) => *n,
                Value::Known(KnownVal::Str(s)) => s.parse::<f64>().unwrap_or(0.0),
                Value::Known(KnownVal::Bool(b)) => if *b { 1.0 } else { 0.0 },
                Value::KnownBool(b) => if *b { 1.0 } else { 0.0 },
                _ => return None,
            };
            let index = index.floor();

            if table_name == and_table || table_name == or_table || table_name == xor_table {
                let lft = (index as i64) >> BINOP_LOOKUP_BITS;
                let rgt = (index as i64) & ((1i64 << BINOP_LOOKUP_BITS) - 1);
                let result = if table_name == and_table {
                    lft & rgt
                } else if table_name == or_table {
                    lft | rgt
                } else {
                    lft ^ rgt
                };
                return Some(Value::Known(KnownVal::Num(result as f64)));
            }

            if table_name == pow2_table {
                let power = index - pow2_offset;
                return Some(Value::Known(KnownVal::Num(2f64.powf(power))));
            }

            None
        };

        // First optimization pass (before post_opt_transform).
        // Matches Python: ctx = optimize(ctx)
        proj = llvm2scratch::optimizer::optimize(
            &proj,
            &compiler_cfg.opt_target,
            100,
            Some(dont_remove.clone()),
            Some(ignore_external_change.clone()),
            Some(&list_lookup),
            &compiler_cfg.opt_passes,
        );

        // Post-optimization transform: merge basic block procedures into
        // branch jump table forever blocks. Matches Python: postOptTransform
        let did_transform = translate::post_opt_transform(&mut proj, &functions, &compiler_cfg);

        // Second optimization pass (after post_opt_transform), only if
        // transform did anything. Matches Python: if did_transform: optimize
        if did_transform {
            proj = llvm2scratch::optimizer::optimize(
                &proj,
                &compiler_cfg.opt_target,
                100,
                Some(dont_remove),
                Some(ignore_external_change),
                Some(&list_lookup),
                &compiler_cfg.opt_passes,
            );
        }
    } else {
        // Even without optimization, still run post_opt_transform to create
        // the branch jump table forever blocks.
        translate::post_opt_transform(&mut proj, &functions, &compiler_cfg);
    }

    if let Some(path) = matches.get_one::<String>("debug-scratch-text") {
        let text = proj.stringify(false);
        if let Err(e) = fs::write(path, text) {
            eprintln!("Error writing debug scratch text '{}': {}", path, e);
            process::exit(1);
        }
    }

    if let Some(path) = matches.get_one::<String>("debug-scratchblocks") {
        let text = proj.stringify(true);
        if let Err(e) = fs::write(path, text) {
            eprintln!("Error writing debug scratchblocks '{}': {}", path, e);
            process::exit(1);
        }
    }

    let mut ctx = project_to_context(&proj);
    if let Err(e) = export_scratch_file(&mut ctx, &output_path.to_string_lossy(), format) {
        eprintln!("Export error: {}", e);
        process::exit(1);
    }

    eprintln!("Successfully compiled to {}", output_path.display());
}
