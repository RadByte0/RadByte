//! Built-in server chat commands (/help, /summon, /heal, /save, /gamemode, /tp, /give, ...).

use super::*;
use crate::gameplay::inventory::PlayerInventory;
use rad_core::packet::play::SimpleSlot;

const ASK_SERVER: &str = "minecraft:ask_server";

struct CommandSpec {
    name: &'static str,
    min_level: u8,
    description: &'static str,
}

const COMMAND_SPECS: &[CommandSpec] = &[
    CommandSpec {
        name: "help",
        min_level: 0,
        description: "Show available commands",
    },
    CommandSpec {
        name: "list",
        min_level: 0,
        description: "List online players",
    },
    CommandSpec {
        name: "tps",
        min_level: 0,
        description: "Show server TPS",
    },
    CommandSpec {
        name: "stats",
        min_level: 0,
        description: "Show player statistics",
    },
    CommandSpec {
        name: "lobby",
        min_level: 0,
        description: "Connect to lobby server",
    },
    CommandSpec {
        name: "hub",
        min_level: 0,
        description: "Connect to hub server",
    },
    CommandSpec {
        name: "dimension",
        min_level: 0,
        description: "Switch dimension (overworld|nether|end)",
    },
    CommandSpec {
        name: "heal",
        min_level: 2,
        description: "Restore health and hunger",
    },
    CommandSpec {
        name: "summon",
        min_level: 2,
        description: "Spawn a mob",
    },
    CommandSpec {
        name: "gamemode",
        min_level: 2,
        description: "Change game mode",
    },
    CommandSpec {
        name: "tp",
        min_level: 2,
        description: "Teleport to coordinates or player",
    },
    CommandSpec {
        name: "give",
        min_level: 2,
        description: "Give an item to a player",
    },
    CommandSpec {
        name: "time",
        min_level: 2,
        description: "Query or modify world time",
    },
    CommandSpec {
        name: "ops",
        min_level: 3,
        description: "List server operators",
    },
    CommandSpec {
        name: "kick",
        min_level: 3,
        description: "Kick a player",
    },
    CommandSpec {
        name: "ban",
        min_level: 3,
        description: "Ban a player",
    },
    CommandSpec {
        name: "pardon",
        min_level: 3,
        description: "Unban a player",
    },
    CommandSpec {
        name: "whitelist",
        min_level: 3,
        description: "Manage server whitelist",
    },
    CommandSpec {
        name: "op",
        min_level: 3,
        description: "Grant operator status",
    },
    CommandSpec {
        name: "deop",
        min_level: 3,
        description: "Revoke operator status",
    },
    CommandSpec {
        name: "save",
        min_level: 4,
        description: "Save modified chunks",
    },
    CommandSpec {
        name: "save-all",
        min_level: 4,
        description: "Save all modified chunks",
    },
    CommandSpec {
        name: "reload",
        min_level: 4,
        description: "Reload server radons and custom recipes",
    },
    CommandSpec {
        name: "stop",
        min_level: 4,
        description: "Stop the server",
    },
];

fn push_node(nodes: &mut Vec<CommandNode>, node: CommandNode) -> VarInt {
    let idx = VarInt(i32::try_from(nodes.len()).unwrap_or(0));
    nodes.push(node);
    idx
}

fn push_literal_subcommands(
    nodes: &mut Vec<CommandNode>,
    cmd_name: &str,
    executable_root: bool,
    subcommands: &[&str],
) {
    let mut children = Vec::with_capacity(subcommands.len());
    for sub in subcommands {
        children.push(push_node(nodes, CommandNode::literal(*sub, true, vec![])));
    }
    let cmd_idx = push_node(
        nodes,
        CommandNode::literal(cmd_name, executable_root, children),
    );
    nodes[0].children.push(cmd_idx);
}

fn push_ask_server_arg_cmd(
    nodes: &mut Vec<CommandNode>,
    cmd_name: &str,
    arg_name: &str,
    executable_root: bool,
) {
    let arg_idx = push_node(
        nodes,
        CommandNode::greedy_arg(arg_name, true, vec![], Some(ASK_SERVER.to_owned())),
    );
    let cmd_idx = push_node(
        nodes,
        CommandNode::literal(cmd_name, executable_root, vec![arg_idx]),
    );
    nodes[0].children.push(cmd_idx);
}

/// Builds the Brigadier command tree sent to clients on Play join, filtered by permission level.
pub(super) fn build_declare_commands_for_level(perm_level: u8) -> DeclareCommands {
    let mut nodes = vec![CommandNode::root(Vec::new())];

    let mut no_arg: Vec<&str> = vec!["help", "list", "tps", "stats", "lobby", "hub"];
    if perm_level >= 2 {
        no_arg.push("heal");
    }
    if perm_level >= 3 {
        no_arg.push("ops");
    }
    if perm_level >= 4 {
        no_arg.extend(["save", "save-all", "reload", "stop"]);
    }
    for name in no_arg {
        let idx = push_node(&mut nodes, CommandNode::literal(name, true, Vec::new()));
        nodes[0].children.push(idx);
    }

    push_literal_subcommands(
        &mut nodes,
        "dimension",
        false,
        &["overworld", "nether", "end"],
    );

    if perm_level >= 2 {
        push_literal_subcommands(
            &mut nodes,
            "gamemode",
            false,
            &["survival", "creative", "adventure", "spectator"],
        );

        // /time <set|add|query>
        let mut set_children = Vec::new();
        for preset in ["day", "noon", "night", "midnight"] {
            set_children.push(push_node(
                &mut nodes,
                CommandNode::literal(preset, true, vec![]),
            ));
        }
        set_children.push(push_node(
            &mut nodes,
            CommandNode::word_arg("ticks", true, vec![], Some(ASK_SERVER.to_owned())),
        ));
        let set_idx = push_node(&mut nodes, CommandNode::literal("set", false, set_children));

        let add_ticks_idx = push_node(
            &mut nodes,
            CommandNode::word_arg("ticks", true, vec![], Some(ASK_SERVER.to_owned())),
        );
        let add_idx = push_node(
            &mut nodes,
            CommandNode::literal("add", false, vec![add_ticks_idx]),
        );

        let mut query_children = Vec::new();
        for kind in ["daytime", "gametime", "day"] {
            query_children.push(push_node(
                &mut nodes,
                CommandNode::literal(kind, true, vec![]),
            ));
        }
        let query_idx = push_node(
            &mut nodes,
            CommandNode::literal("query", true, query_children),
        );
        let time_idx = push_node(
            &mut nodes,
            CommandNode::literal("time", false, vec![set_idx, add_idx, query_idx]),
        );
        nodes[0].children.push(time_idx);

        for (cmd_name, arg_name) in [("summon", "mob"), ("tp", "coords"), ("give", "item")] {
            push_ask_server_arg_cmd(&mut nodes, cmd_name, arg_name, false);
        }
    }

    if perm_level >= 3 {
        // /whitelist <on|off|reload|list|add <player>|remove <player>>
        let mut wl_children = Vec::new();
        for sub in ["on", "off", "reload", "list"] {
            wl_children.push(push_node(
                &mut nodes,
                CommandNode::literal(sub, true, vec![]),
            ));
        }
        for sub in ["add", "remove"] {
            let player_idx = push_node(
                &mut nodes,
                CommandNode::word_arg("player", true, vec![], Some(ASK_SERVER.to_owned())),
            );
            wl_children.push(push_node(
                &mut nodes,
                CommandNode::literal(sub, false, vec![player_idx]),
            ));
        }
        let wl_idx = push_node(
            &mut nodes,
            CommandNode::literal("whitelist", false, wl_children),
        );
        nodes[0].children.push(wl_idx);

        // /op <list|player [level]>
        let op_list_idx = push_node(&mut nodes, CommandNode::literal("list", true, vec![]));
        let op_player_idx = push_node(
            &mut nodes,
            CommandNode::greedy_arg("player", true, vec![], Some(ASK_SERVER.to_owned())),
        );
        let op_idx = push_node(
            &mut nodes,
            CommandNode::literal("op", false, vec![op_list_idx, op_player_idx]),
        );
        nodes[0].children.push(op_idx);

        for (cmd_name, arg_name) in [
            ("kick", "player"),
            ("ban", "player"),
            ("pardon", "player"),
            ("deop", "player"),
        ] {
            push_ask_server_arg_cmd(&mut nodes, cmd_name, arg_name, false);
        }
    }

    DeclareCommands {
        nodes,
        root_index: VarInt(0),
    }
}

/// Builds the full Brigadier command tree (level 4).
#[allow(dead_code)]
pub(super) fn build_declare_commands() -> DeclareCommands {
    build_declare_commands_for_level(4)
}

fn suggest_match(text: impl Into<String>, tooltip: Option<&str>) -> CommandSuggestionMatch {
    CommandSuggestionMatch {
        text: text.into(),
        tooltip: tooltip.map(|t| NbtTag::String(t.to_owned())),
    }
}

fn collect_matching_literals<'a>(
    candidates: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    prefix: &str,
) -> Vec<CommandSuggestionMatch> {
    let lower = prefix.to_ascii_lowercase();
    let mut out: Vec<CommandSuggestionMatch> = candidates
        .into_iter()
        .filter(|(name, _)| name.to_ascii_lowercase().starts_with(&lower))
        .map(|(name, tip)| suggest_match(name, tip))
        .collect();
    out.sort_by(|a, b| a.text.cmp(&b.text));
    out
}

/// Computes permission-filtered command and argument suggestions for [`CommandSuggestionsRequest`].
pub(super) async fn build_command_suggestions(
    state: &ServerState,
    username: &str,
    sender_uuid: Option<Uuid>,
    req: &CommandSuggestionsRequest,
) -> CommandSuggestions {
    let perm_level = state.permission_level(username, sender_uuid).await;
    let raw = req.text.as_str();
    let leading_slash = raw.starts_with('/');
    let trimmed_start = raw.strip_prefix('/').unwrap_or(raw);
    let base_offset = usize::from(leading_slash);

    let (start_byte, matches) = if !trimmed_start.contains(' ') {
        let matches = collect_matching_literals(
            COMMAND_SPECS
                .iter()
                .filter(|spec| perm_level >= spec.min_level)
                .map(|spec| (spec.name, Some(spec.description))),
            trimmed_start,
        );
        (base_offset, matches)
    } else {
        let mut parts = trimmed_start.splitn(2, ' ');
        let cmd_name = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let rest = parts.next().unwrap_or("");
        let allowed = COMMAND_SPECS
            .iter()
            .find(|s| s.name == cmd_name)
            .is_some_and(|s| perm_level >= s.min_level);

        if !allowed {
            (raw.len(), Vec::new())
        } else {
            let trailing_space = rest.ends_with(' ');
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            let current_token = if trailing_space {
                ""
            } else {
                tokens.last().copied().unwrap_or("")
            };
            let arg_index = if trailing_space {
                tokens.len()
            } else {
                tokens.len().saturating_sub(1)
            };
            let replace_start = raw.len().saturating_sub(current_token.len());

            let matches = match cmd_name.as_str() {
                "dimension" if arg_index == 0 => collect_matching_literals(
                    [
                        ("overworld", Some("Normal world")),
                        ("nether", Some("The Nether")),
                        ("end", Some("The End")),
                    ],
                    current_token,
                ),
                "gamemode" if arg_index == 0 => collect_matching_literals(
                    [
                        ("survival", Some("Game mode 0")),
                        ("creative", Some("Game mode 1")),
                        ("adventure", Some("Game mode 2")),
                        ("spectator", Some("Game mode 3")),
                    ],
                    current_token,
                ),
                "time" if arg_index == 0 => collect_matching_literals(
                    [
                        ("set", Some("Set world time")),
                        ("add", Some("Advance world time")),
                        ("query", Some("Query current time")),
                    ],
                    current_token,
                ),
                "time"
                    if arg_index == 1
                        && tokens
                            .first()
                            .is_some_and(|t| t.eq_ignore_ascii_case("set")) =>
                {
                    collect_matching_literals(
                        [
                            ("day", Some("1000 ticks")),
                            ("noon", Some("6000 ticks")),
                            ("night", Some("13000 ticks")),
                            ("midnight", Some("18000 ticks")),
                        ],
                        current_token,
                    )
                }
                "time"
                    if arg_index == 1
                        && tokens
                            .first()
                            .is_some_and(|t| t.eq_ignore_ascii_case("query")) =>
                {
                    collect_matching_literals(
                        [
                            ("daytime", Some("Time of day (0..23999)")),
                            ("gametime", Some("Total world ticks")),
                            ("day", Some("Elapsed days")),
                        ],
                        current_token,
                    )
                }
                "summon" if arg_index == 0 => collect_matching_literals(
                    [
                        ("zombie", Some("Hostile undead mob")),
                        ("skeleton", Some("Hostile ranged mob")),
                        ("pig", Some("Passive farm mob")),
                    ],
                    current_token,
                ),
                "whitelist" if arg_index == 0 => collect_matching_literals(
                    [
                        ("on", Some("Enable whitelist")),
                        ("off", Some("Disable whitelist")),
                        ("reload", Some("Reload access lists from disk")),
                        ("list", Some("Show whitelisted players")),
                        ("add", Some("Add player to whitelist")),
                        ("remove", Some("Remove player from whitelist")),
                    ],
                    current_token,
                ),
                "whitelist"
                    if arg_index == 1
                        && tokens
                            .first()
                            .is_some_and(|t| t.eq_ignore_ascii_case("remove")) =>
                {
                    let access = state.access.read().await;
                    let names: Vec<String> = access
                        .whitelist_entries
                        .iter()
                        .map(|e| e.name.clone())
                        .collect();
                    collect_matching_literals(
                        names
                            .iter()
                            .map(|n| (n.as_str(), Some("Whitelisted player"))),
                        current_token,
                    )
                }
                "deop" if arg_index == 0 => {
                    let access = state.access.read().await;
                    let names: Vec<String> =
                        access.op_entries.iter().map(|e| e.name.clone()).collect();
                    collect_matching_literals(
                        names.iter().map(|n| (n.as_str(), Some("Server operator"))),
                        current_token,
                    )
                }
                "pardon" if arg_index == 0 => {
                    let access = state.access.read().await;
                    let names: Vec<String> = access.banned.iter().cloned().collect();
                    collect_matching_literals(
                        names.iter().map(|n| (n.as_str(), Some("Banned player"))),
                        current_token,
                    )
                }
                "kick" | "ban" | "op" | "whitelist" if arg_index == 0 || arg_index == 1 => {
                    let players = state.players.read().await;
                    let names: Vec<String> = players.values().map(|p| p.username.clone()).collect();
                    let mut opts: Vec<(&str, Option<&str>)> = names
                        .iter()
                        .map(|n| (n.as_str(), Some("Online player")))
                        .collect();
                    if cmd_name == "op" && arg_index == 0 {
                        opts.push(("list", Some("List operators")));
                    }
                    collect_matching_literals(opts, current_token)
                }
                "tp" if arg_index == 0 => {
                    let players = state.players.read().await;
                    let names: Vec<String> = players.values().map(|p| p.username.clone()).collect();
                    let mut opts: Vec<(&str, Option<&str>)> = vec![
                        ("@s", Some("Current player")),
                        ("@p", Some("Nearest player")),
                        ("@a", Some("All players")),
                        ("~ ~ ~", Some("Relative coordinates")),
                    ];
                    for n in &names {
                        opts.push((n.as_str(), Some("Online player")));
                    }
                    collect_matching_literals(opts, current_token)
                }
                "give" => {
                    let pkg = &state.version_package;
                    let lower = current_token.to_ascii_lowercase();
                    let mut out = Vec::new();
                    if arg_index == 0 {
                        let players = state.players.read().await;
                        let names: Vec<String> =
                            players.values().map(|p| p.username.clone()).collect();
                        let mut p_opts: Vec<(&str, Option<&str>)> = vec![
                            ("@s", Some("Current player")),
                            ("@p", Some("Nearest player")),
                            ("@a", Some("All players")),
                        ];
                        for n in &names {
                            p_opts.push((n.as_str(), Some("Online player")));
                        }
                        out.extend(collect_matching_literals(p_opts, current_token));
                    }
                    if arg_index <= 1 {
                        let mut item_matches: Vec<CommandSuggestionMatch> = pkg
                            .item_names()
                            .filter(|name| {
                                name.to_ascii_lowercase().starts_with(&lower)
                                    || name.strip_prefix("minecraft:").is_some_and(|short| {
                                        short.to_ascii_lowercase().starts_with(&lower)
                                    })
                            })
                            .take(50)
                            .map(|name| suggest_match(name, Some("Item")))
                            .collect();
                        item_matches.sort_by(|a, b| a.text.cmp(&b.text));
                        out.extend(item_matches);
                    }
                    out
                }
                _ => Vec::new(),
            };
            (replace_start, matches)
        }
    };

    let start_chars = raw[..start_byte].chars().count();
    let len_chars = raw[start_byte..].chars().count();
    CommandSuggestions {
        id: req.id,
        start: VarInt(i32::try_from(start_chars).unwrap_or(0)),
        length: VarInt(i32::try_from(len_chars).unwrap_or(0)),
        matches,
    }
}

/// Handles a serverbound [`CommandSuggestionsRequest`] and sends [`CommandSuggestions`] to the player.
pub(super) async fn handle_command_suggestions(
    state: &ServerState,
    tx: &OutboundSender,
    username: &str,
    req: &CommandSuggestionsRequest,
) {
    let sender_uuid = {
        let players = state.players.read().await;
        players
            .values()
            .find(|p| p.tx.same_channel(tx))
            .map(|p| p.uuid)
    };
    let resp = build_command_suggestions(state, username, sender_uuid, req).await;
    if let Ok(bytes) = encode_to_bytes(&resp) {
        let _ = tx.try_send(bytes);
    }
}

fn parse_game_mode(raw: &str) -> Option<u8> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "0" | "survival" => Some(0),
        "1" | "creative" => Some(1),
        "2" | "adventure" => Some(2),
        "3" | "spectator" => Some(3),
        _ => None,
    }
}

fn parse_item_id(raw: &str, pkg: &rad_core::version_package::LoadedVersionPackage) -> Option<i32> {
    if let Some(id) = pkg.item_id_by_name(raw) {
        return Some(id);
    }
    match raw.trim().to_ascii_lowercase().as_str() {
        "planks" => pkg.item_id_by_name("minecraft:oak_planks"),
        other => other
            .parse::<i32>()
            .ok()
            .filter(|&id| id > 0 && pkg.item_name_by_id(id).is_some()),
    }
}

fn give_item_to_inventory(inv: &mut PlayerInventory, item_id: i32, mut count: i32) -> i32 {
    let mut given = 0;
    // Pass 1: stack onto existing slots (hotbar first, then main inventory)
    for idx in (36..45).chain(9..36) {
        if count <= 0 {
            break;
        }
        let slot = &mut inv.slots[idx];
        if slot.item_id == item_id && slot.count > 0 && slot.count < 64 {
            let add = (64 - slot.count).min(count);
            slot.count += add;
            count -= add;
            given += add;
        }
    }
    // Pass 2: fill empty slots
    for idx in (36..45).chain(9..36) {
        if count <= 0 {
            break;
        }
        let slot = &mut inv.slots[idx];
        if slot.count <= 0 || slot.item_id == 0 {
            let add = count.min(64);
            *slot = SimpleSlot {
                count: add,
                item_id,
            };
            count -= add;
            given += add;
        }
    }
    given
}

fn reply_cmd(tx: Option<&OutboundSender>, msg: String) {
    if let Some(sender) = tx {
        send_chat_to(sender, msg);
    } else {
        tracing::info!(target: "rad_server::console", "{msg}");
    }
}

/// Executes a console command with full level-4 administrative permissions.
/// Returns `true` if the console requested server shutdown (`stop`/`quit`/`exit`).
pub(crate) async fn handle_console_command(state: &ServerState, raw_cmd: &str) -> bool {
    let trimmed = raw_cmd.trim().trim_start_matches('/');
    if matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "stop" | "quit" | "exit"
    ) {
        return true;
    }
    let normalized = if trimmed.eq_ignore_ascii_case("save-all") {
        "save"
    } else {
        trimmed
    };
    handle_server_command_inner(state, None, "CONSOLE", normalized, None, 4).await;
    false
}

pub(super) async fn handle_server_command(
    state: &ServerState,
    tx: &OutboundSender,
    username: &str,
    raw_cmd: &str,
    inventory: Option<&std::sync::Mutex<PlayerInventory>>,
) {
    let sender_uuid = {
        let players = state.players.read().await;
        players
            .values()
            .find(|p| p.tx.same_channel(tx))
            .map(|p| p.uuid)
    };
    let perm_level = state.permission_level(username, sender_uuid).await;
    handle_server_command_inner(state, Some(tx), username, raw_cmd, inventory, perm_level).await;
}

async fn handle_server_command_inner(
    state: &ServerState,
    tx: Option<&OutboundSender>,
    _username: &str,
    raw_cmd: &str,
    inventory: Option<&std::sync::Mutex<PlayerInventory>>,
    perm_level: u8,
) {
    let cmd = raw_cmd.trim();
    if cmd.eq_ignore_ascii_case("help") {
        let mut cmds = vec!["/help", "/list", "/tps", "/stats", "/dimension", "/lobby"];
        if perm_level >= 2 {
            cmds.extend(["/heal", "/summon", "/gamemode", "/tp", "/give"]);
        }
        if perm_level >= 3 {
            cmds.extend([
                "/kick",
                "/ban",
                "/pardon",
                "/whitelist",
                "/op",
                "/deop",
                "/ops",
            ]);
        }
        if perm_level >= 4 {
            cmds.extend(["/save", "/reload", "/stop"]);
        }
        reply_cmd(tx, format!("RadServer commands: {}", cmds.join(", ")));
    } else if let Some(mob_name) = cmd.strip_prefix("summon ").map(str::trim) {
        if perm_level < 2 {
            reply_cmd(tx, "You do not have permission to use /summon.".into());
            return;
        }
        if let Some(kind) = MobKind::parse(mob_name) {
            let spawn_result = {
                let mut entities = state.entities.write().await;
                if entities.all_mobs().count() >= state.config.max_mobs {
                    None
                } else {
                    let eid = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
                    let pkt = entities
                        .spawn_mob(
                            eid,
                            kind,
                            state.config.spawn_x + 2.0,
                            state.config.spawn_y,
                            state.config.spawn_z + 2.0,
                        )
                        .to_spawn_packet_with(&state.registries.entities);
                    Some((eid, pkt))
                }
            };
            if let Some((eid, spawn_pkt)) = spawn_result {
                state.broadcast_except(&spawn_pkt, None).await;
                reply_cmd(tx, format!("Summoned {mob_name} (id={eid})."));
            } else {
                reply_cmd(
                    tx,
                    format!("Mob cap reached ({} max mobs).", state.config.max_mobs),
                );
            }
        } else {
            reply_cmd(tx, "Unknown mob. Try: zombie, pig, skeleton".into());
        }
    } else if cmd.eq_ignore_ascii_case("heal") {
        if perm_level < 2 {
            reply_cmd(tx, "You do not have permission to use /heal.".into());
            return;
        }
        let Some(sender_tx) = tx else {
            reply_cmd(tx, "Console cannot /heal itself.".into());
            return;
        };
        let mut players = state.players.write().await;
        for p in players.values_mut() {
            if p.tx.same_channel(sender_tx) {
                p.hp = 20.0;
                p.food = 20;
                p.saturation = 5.0;
                p.dead = false;
                if let Ok(bytes) = encode_to_bytes(&SetHealth {
                    health: 20.0,
                    food: VarInt(20),
                    food_saturation: 5.0,
                }) {
                    let _ = sender_tx.try_send(bytes);
                }
                break;
            }
        }
    } else if cmd.eq_ignore_ascii_case("save") || cmd.eq_ignore_ascii_case("save-all") {
        if perm_level < 4 {
            reply_cmd(tx, "You do not have permission to use /save.".into());
            return;
        }
        match state.save_all().await {
            Ok(n) => reply_cmd(tx, format!("Saved {n} modified chunk(s) to Anvil (.mca).")),
            Err(e) => reply_cmd(tx, format!("Save failed: {e}")),
        }
    } else if cmd.eq_ignore_ascii_case("list") {
        let players = state.players.read().await;
        let names: Vec<&str> = players.values().map(|p| p.username.as_str()).collect();
        reply_cmd(
            tx,
            format!("Online ({}): {}", names.len(), names.join(", ")),
        );
    } else if cmd.eq_ignore_ascii_case("tps") {
        let ticks = state.tick_count.load(Ordering::Relaxed);
        let last_us = state.last_tick_us.load(Ordering::Relaxed);
        reply_cmd(
            tx,
            format!(
                "Target TPS: {} | Total ticks: {} | Last tick: {:.2} ms",
                state.config.tps,
                ticks,
                last_us as f64 / 1000.0
            ),
        );
    } else if let Some(mode_arg) = cmd.strip_prefix("gamemode ").map(str::trim) {
        if perm_level < 2 {
            reply_cmd(tx, "You do not have permission to use /gamemode.".into());
            return;
        }
        let mut parts = mode_arg.split_whitespace();
        let Some(mode) = parts.next().and_then(parse_game_mode) else {
            reply_cmd(
                tx,
                "Invalid game mode. Try: survival, creative, adventure, spectator".into(),
            );
            return;
        };
        let target_sel = parts.next().unwrap_or("@s");
        let target_ids = resolve_target_entity_ids(state, tx, target_sel).await;
        if target_ids.is_empty() {
            reply_cmd(tx, format!("No player matched selector '{target_sel}'."));
            return;
        }
        let updated_entries = {
            let mut players = state.players.write().await;
            let mut entries = Vec::with_capacity(target_ids.len());
            for p in players.values_mut() {
                if target_ids.contains(&p.entity_id) {
                    p.game_mode = mode;
                    if let Ok(bytes) = encode_to_bytes(&GameEvent {
                        event: GameEvent::CHANGE_GAME_MODE,
                        value: f32::from(mode),
                    }) {
                        let _ = p.tx.try_send(bytes);
                    }
                    if let Some(content) = gamemode_notice(mode)
                        && let Ok(bytes) = encode_to_bytes(&SystemChatMessage {
                            content,
                            overlay: false,
                        })
                    {
                        let _ = p.tx.try_send(bytes);
                    }
                    entries.push(PlayerInfoAddEntry {
                        uuid: p.uuid,
                        name: p.username.clone(),
                        properties: p.properties.clone(),
                        game_mode: VarInt(i32::from(mode)),
                        listed: true,
                        latency: VarInt(0),
                    });
                }
            }
            entries
        };
        if !updated_entries.is_empty() {
            state
                .broadcast_except(
                    &PlayerInfoUpdate {
                        entries: updated_entries,
                    },
                    None,
                )
                .await;
        }
    } else if let Some(tp_arg) = cmd.strip_prefix("tp ").map(str::trim) {
        if perm_level < 2 {
            reply_cmd(tx, "You do not have permission to use /tp.".into());
            return;
        }
        let parts: Vec<&str> = tp_arg.split_whitespace().collect();
        let (target_sel, coord_slice) = if parts.len() == 4 {
            (parts[0], &parts[1..4])
        } else if parts.len() == 3 {
            ("@s", &parts[0..3])
        } else {
            reply_cmd(
                tx,
                "Usage: /tp [target|@a|@p|@s] <x> <y> <z> (y in -64..=320)".into(),
            );
            return;
        };
        let coords = match (
            coord_slice[0].parse::<f64>(),
            coord_slice[1].parse::<f64>(),
            coord_slice[2].parse::<f64>(),
        ) {
            (Ok(x), Ok(y), Ok(z))
                if x.is_finite()
                    && y.is_finite()
                    && z.is_finite()
                    && x.abs() <= 30_000_000.0
                    && z.abs() <= 30_000_000.0
                    && (-64.0..=320.0).contains(&y) =>
            {
                Some((x, y, z))
            }
            _ => None,
        };
        let Some((x, y, z)) = coords else {
            reply_cmd(
                tx,
                "Usage: /tp [target|@a|@p|@s] <x> <y> <z> (y in -64..=320)".into(),
            );
            return;
        };
        let target_ids = resolve_target_entity_ids(state, tx, target_sel).await;
        if target_ids.is_empty() {
            reply_cmd(tx, format!("No player matched selector '{target_sel}'."));
            return;
        }
        let mut players = state.players.write().await;
        for p in players.values_mut() {
            if target_ids.contains(&p.entity_id) {
                let tp_id = 1000 + p.entity_id;
                p.x = x;
                p.y = y;
                p.z = z;
                p.fall_start_y = None;
                p.pending_teleport_id = Some(tp_id);
                if let Ok(bytes) = encode_to_bytes(&SynchronizePlayerPosition {
                    teleport_id: VarInt(tp_id),
                    x,
                    y,
                    z,
                    vx: 0.0,
                    vy: 0.0,
                    vz: 0.0,
                    yaw: p.yaw,
                    pitch: p.pitch,
                    flags: 0,
                }) {
                    let _ = p.tx.try_send(bytes);
                }
                send_chat_to(&p.tx, format!("Teleported to {x:.1}, {y:.1}, {z:.1}."));
            }
        }
    } else if let Some(give_arg) = cmd.strip_prefix("give ").map(str::trim) {
        if perm_level < 2 {
            reply_cmd(tx, "You do not have permission to use /give.".into());
            return;
        }
        let mut parts = give_arg.split_whitespace();
        let Some(item_raw) = parts.next() else {
            reply_cmd(tx, "Usage: /give <item> [count]".into());
            return;
        };
        let count = parts
            .next()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(1)
            .clamp(1, 64);
        let Some(item_id) = parse_item_id(item_raw, &state.version_package) else {
            reply_cmd(
                tx,
                "Unknown item. Try: stone, oak_planks, chest, stick, dirt, cobblestone, or numeric id"
                    .into(),
            );
            return;
        };
        let Some(inv_lock) = inventory else {
            reply_cmd(tx, "Inventory unavailable for /give.".into());
            return;
        };
        let (given, pkt) = {
            let mut inv = inv_lock.lock().unwrap();
            let given = give_item_to_inventory(&mut inv, item_id, count);
            let pkt = if given > 0 {
                encode_to_bytes(&inv.to_container_set_content_stacks()).ok()
            } else {
                None
            };
            (given, pkt)
        };
        if given > 0 {
            if let Some(sender_tx) = tx
                && let Some(bytes) = pkt
            {
                let _ = sender_tx.try_send(bytes);
            }
            reply_cmd(tx, format!("Gave {given}x item #{item_id}."));
        } else {
            reply_cmd(tx, "Inventory is full.".into());
        }
    } else if let Some(kick_arg) = cmd.strip_prefix("kick ") {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /kick.".into());
            return;
        }
        let mut parts = kick_arg.splitn(2, char::is_whitespace);
        let target = parts.next().unwrap_or("").trim();
        let reason = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Kicked by an operator.");
        if target.is_empty() {
            reply_cmd(tx, "Usage: /kick <player> [reason]".into());
            return;
        }
        if kick_online_player(state, target, reason).await {
            reply_cmd(tx, format!("Kicked {target}."));
        } else {
            reply_cmd(tx, format!("Player '{target}' is not online."));
        }
    } else if let Some(ban_arg) = cmd.strip_prefix("ban ") {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /ban.".into());
            return;
        }
        let mut parts = ban_arg.splitn(2, char::is_whitespace);
        let target = parts.next().unwrap_or("").trim();
        let reason = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("You are banned from this server.");
        if target.is_empty() {
            reply_cmd(tx, "Usage: /ban <player> [reason]".into());
            return;
        }
        let key = target.to_ascii_lowercase();
        let inserted = {
            let mut access = state.access.write().await;
            let added = access.banned.insert(key);
            if added {
                access.mark_dirty();
            }
            added
        };
        let _ = kick_online_player(state, target, reason).await;
        if inserted {
            reply_cmd(tx, format!("Banned {target}."));
        } else {
            reply_cmd(tx, format!("{target} is already banned."));
        }
    } else if let Some(pardon_arg) = cmd.strip_prefix("pardon ") {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /pardon.".into());
            return;
        }
        let target = pardon_arg.trim();
        if target.is_empty() {
            reply_cmd(tx, "Usage: /pardon <player>".into());
            return;
        }
        let key = target.to_ascii_lowercase();
        let removed = {
            let mut access = state.access.write().await;
            let ok = access.banned.remove(&key);
            if ok {
                access.mark_dirty();
            }
            ok
        };
        if removed {
            reply_cmd(tx, format!("Unbanned {target}."));
        } else {
            reply_cmd(tx, format!("{target} is not banned."));
        }
    } else if cmd.eq_ignore_ascii_case("whitelist")
        || cmd.to_ascii_lowercase().starts_with("whitelist ")
    {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /whitelist.".into());
            return;
        }
        let wl_arg = cmd.split_once(' ').map_or("", |(_, rest)| rest);
        let mut parts = wl_arg.split_whitespace();
        match parts.next() {
            Some("on") => {
                {
                    let mut access = state.access.write().await;
                    if !access.whitelist_enabled {
                        access.whitelist_enabled = true;
                        access.mark_dirty();
                    }
                }
                let kicked = state.enforce_whitelist_online().await;
                if kicked > 0 {
                    reply_cmd(
                        tx,
                        format!(
                            "Whitelist is now turned on (kicked {kicked} non-whitelisted player(s))."
                        ),
                    );
                } else {
                    reply_cmd(tx, "Whitelist is now turned on.".into());
                }
            }
            Some("off") => {
                let mut access = state.access.write().await;
                if access.whitelist_enabled {
                    access.whitelist_enabled = false;
                    access.mark_dirty();
                }
                reply_cmd(tx, "Whitelist is now turned off.".into());
            }
            Some("reload") => {
                let world_dir = state.world.read().await.world_dir().map(PathBuf::from);
                if let Some(dir) = world_dir {
                    let loaded = AccessControl::load_from_dir(&dir);
                    {
                        let mut access = state.access.write().await;
                        access.whitelist = loaded.whitelist;
                        access.whitelist_entries = loaded.whitelist_entries;
                    }
                    let kicked = state.enforce_whitelist_online().await;
                    if kicked > 0 {
                        reply_cmd(
                            tx,
                            format!(
                                "Reloaded whitelist (kicked {kicked} non-whitelisted player(s))."
                            ),
                        );
                    } else {
                        reply_cmd(tx, "Reloaded whitelist from whitelist.json.".into());
                    }
                } else {
                    reply_cmd(
                        tx,
                        "Whitelist reload skipped: no world directory configured.".into(),
                    );
                }
            }
            Some("list") => {
                let access = state.access.read().await;
                let mut entries: Vec<String> = access
                    .whitelist_entries
                    .iter()
                    .map(|e| {
                        if let Some(u) = e.uuid {
                            format!("{} ({u})", e.name)
                        } else {
                            e.name.clone()
                        }
                    })
                    .collect();
                for legacy in &access.whitelist {
                    if !access
                        .whitelist_entries
                        .iter()
                        .any(|e| e.name.eq_ignore_ascii_case(legacy))
                    {
                        entries.push(legacy.clone());
                    }
                }
                entries.sort_unstable();
                let status = if access.whitelist_enabled {
                    "on"
                } else {
                    "off"
                };
                reply_cmd(
                    tx,
                    format!(
                        "Whitelist ({status}, {}): {}",
                        entries.len(),
                        if entries.is_empty() {
                            "<empty>".to_owned()
                        } else {
                            entries.join(", ")
                        }
                    ),
                );
            }
            Some("add") => {
                let Some(target) = parts.next() else {
                    reply_cmd(tx, "Usage: /whitelist add <player|uuid>".into());
                    return;
                };
                let (resolved_uuid, resolved_name) = {
                    let players = state.players.read().await;
                    if let Ok(u) = Uuid::parse_str(target) {
                        if let Some(p) = players.values().find(|p| p.uuid == u) {
                            (Some(p.uuid), p.username.clone())
                        } else {
                            (Some(u), u.to_string())
                        }
                    } else if let Some(p) = players
                        .values()
                        .find(|p| p.username.eq_ignore_ascii_case(target))
                    {
                        (Some(p.uuid), p.username.clone())
                    } else {
                        (None, target.to_owned())
                    }
                };
                let binding_uuid =
                    if state.trusts_forwarded_identity() || Uuid::parse_str(target).is_ok() {
                        resolved_uuid
                    } else {
                        None
                    };
                let mut access = state.access.write().await;
                if access.add_whitelist(binding_uuid, resolved_name.clone()) {
                    reply_cmd(tx, format!("Added {resolved_name} to the whitelist."));
                } else {
                    reply_cmd(tx, format!("{resolved_name} is already whitelisted."));
                }
            }
            Some("remove") => {
                let Some(target) = parts.next() else {
                    reply_cmd(tx, "Usage: /whitelist remove <player|uuid>".into());
                    return;
                };
                let mut access = state.access.write().await;
                if access.remove_whitelist(target) {
                    reply_cmd(tx, format!("Removed {target} from the whitelist."));
                } else {
                    reply_cmd(tx, format!("{target} is not on the whitelist."));
                }
            }
            _ => {
                reply_cmd(
                    tx,
                    "Usage: /whitelist <on|off|list|reload|add <player|uuid>|remove <player|uuid>>"
                        .into(),
                );
            }
        }
    } else if cmd.eq_ignore_ascii_case("ops") {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /ops.".into());
            return;
        }
        let access = state.access.read().await;
        let mut entries: Vec<String> = state
            .config
            .admins
            .iter()
            .map(|a| format!("{a} (level 4, config)"))
            .collect();
        for e in &access.op_entries {
            if let Some(u) = e.uuid {
                entries.push(format!("{} (level {}, {u})", e.name, e.level));
            } else {
                entries.push(format!("{} (level {})", e.name, e.level));
            }
        }
        entries.sort_unstable();
        reply_cmd(
            tx,
            format!(
                "Operators ({}): {}",
                entries.len(),
                if entries.is_empty() {
                    "<none>".to_owned()
                } else {
                    entries.join(", ")
                }
            ),
        );
    } else if let Some(op_arg) = cmd.strip_prefix("op ").or_else(|| cmd.strip_prefix("OP ")) {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /op.".into());
            return;
        }
        let mut parts = op_arg.split_whitespace();
        let Some(target) = parts.next() else {
            reply_cmd(tx, "Usage: /op <player|uuid> [1..4]".into());
            return;
        };
        if target.eq_ignore_ascii_case("list") {
            let access = state.access.read().await;
            let mut entries: Vec<String> = state
                .config
                .admins
                .iter()
                .map(|a| format!("{a} (level 4, config)"))
                .collect();
            for e in &access.op_entries {
                entries.push(format!("{} (level {})", e.name, e.level));
            }
            entries.sort_unstable();
            reply_cmd(
                tx,
                format!(
                    "Operators ({}): {}",
                    entries.len(),
                    if entries.is_empty() {
                        "<none>".to_owned()
                    } else {
                        entries.join(", ")
                    }
                ),
            );
            return;
        }
        let req_level = match parts.next() {
            Some(lvl_str) => match lvl_str.parse::<u8>().ok().filter(|&l| (1..=4).contains(&l)) {
                Some(l) => l,
                None => {
                    reply_cmd(tx, "Invalid op level; expected 1..=4.".into());
                    return;
                }
            },
            None => state.config.op_permission_level,
        };
        if req_level > perm_level {
            reply_cmd(
                tx,
                format!("Cannot grant op level {req_level} above your own level ({perm_level})."),
            );
            return;
        }
        let (resolved_uuid, resolved_name, target_tx) = {
            let players = state.players.read().await;
            if let Ok(u) = Uuid::parse_str(target) {
                if let Some(p) = players.values().find(|p| p.uuid == u) {
                    (
                        Some(p.uuid),
                        p.username.clone(),
                        Some((p.entity_id, p.tx.clone())),
                    )
                } else {
                    (Some(u), u.to_string(), None)
                }
            } else if let Some(p) = players
                .values()
                .find(|p| p.username.eq_ignore_ascii_case(target))
            {
                (
                    Some(p.uuid),
                    p.username.clone(),
                    Some((p.entity_id, p.tx.clone())),
                )
            } else {
                (None, target.to_owned(), None)
            }
        };
        let binding_uuid = if state.trusts_forwarded_identity() || Uuid::parse_str(target).is_ok() {
            resolved_uuid
        } else {
            None
        };
        let changed =
            state
                .access
                .write()
                .await
                .add_op(binding_uuid, resolved_name.clone(), req_level);
        if changed {
            if let Some((target_id, ttx)) = target_tx {
                let new_lvl = state.permission_level(&resolved_name, resolved_uuid).await;
                if let Ok(bytes) = encode_to_bytes(&PermissionEvent {
                    entity_id: target_id,
                    level: new_lvl,
                }) {
                    let _ = ttx.try_send(bytes);
                }
                if let Ok(bytes) = encode_to_bytes(&build_declare_commands_for_level(new_lvl)) {
                    let _ = ttx.try_send(bytes);
                }
                send_chat_to(
                    &ttx,
                    format!("You are now a server operator (level {req_level})."),
                );
            }
            reply_cmd(
                tx,
                format!("Made {resolved_name} a server operator (level {req_level})."),
            );
        } else {
            reply_cmd(
                tx,
                format!("{resolved_name} is already an operator (level {req_level})."),
            );
        }
    } else if let Some(deop_arg) = cmd
        .strip_prefix("deop ")
        .or_else(|| cmd.strip_prefix("DEOP "))
    {
        if perm_level < 3 {
            reply_cmd(tx, "You do not have permission to use /deop.".into());
            return;
        }
        let target = deop_arg.trim();
        if target.is_empty() {
            reply_cmd(tx, "Usage: /deop <player|uuid>".into());
            return;
        }
        let removed = state.access.write().await.remove_op(target);
        if removed {
            let target_info = {
                let players = state.players.read().await;
                let parsed_uuid = Uuid::parse_str(target).ok();
                players
                    .values()
                    .find(|p| {
                        parsed_uuid.is_some_and(|u| p.uuid == u)
                            || p.username.eq_ignore_ascii_case(target)
                    })
                    .map(|p| (p.username.clone(), p.uuid, p.entity_id, p.tx.clone()))
            };
            if let Some((tname, tuuid, target_id, ttx)) = target_info {
                let new_lvl = state.permission_level(&tname, Some(tuuid)).await;
                if let Ok(bytes) = encode_to_bytes(&PermissionEvent {
                    entity_id: target_id,
                    level: new_lvl,
                }) {
                    let _ = ttx.try_send(bytes);
                }
                if let Ok(bytes) = encode_to_bytes(&build_declare_commands_for_level(new_lvl)) {
                    let _ = ttx.try_send(bytes);
                }
                send_chat_to(&ttx, "You are no longer a server operator.".into());
            }
            reply_cmd(tx, format!("Removed operator status from {target}."));
        } else {
            reply_cmd(tx, format!("{target} is not a runtime operator."));
        }
    } else if cmd
        .split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case("time"))
    {
        if perm_level < 2 {
            reply_cmd(tx, "Permission denied.".into());
            return;
        }
        let mut parts = cmd.split_whitespace().skip(1);
        let sub = parts.next().unwrap_or("");
        if sub.eq_ignore_ascii_case("query") {
            let kind = parts.next().unwrap_or("daytime");
            let (game_time, day_time) = state.sync_level_time().await;
            let value = if kind.eq_ignore_ascii_case("gametime") {
                game_time
            } else if kind.eq_ignore_ascii_case("day") {
                day_time / 24000
            } else {
                day_time % 24000
            };
            reply_cmd(tx, format!("The time is {value}"));
        } else if sub.eq_ignore_ascii_case("set") {
            let Some(raw) = parts.next() else {
                reply_cmd(
                    tx,
                    "Usage: /time set <day|noon|night|midnight|ticks>".into(),
                );
                return;
            };
            let target_time = match raw.to_ascii_lowercase().as_str() {
                "day" => Some(1000),
                "noon" => Some(6000),
                "night" => Some(13000),
                "midnight" => Some(18000),
                _ => raw.parse::<i64>().ok().filter(|&v| v >= 0),
            };
            let Some(new_day_time) = target_time else {
                reply_cmd(
                    tx,
                    "Usage: /time set <day|noon|night|midnight|ticks>".into(),
                );
                return;
            };
            let (game_time, day_time) = state.set_day_time(new_day_time).await;
            state
                .broadcast_except(
                    &rad_core::packet::play::SetTime::overworld(game_time, day_time, 1.0),
                    None,
                )
                .await;
            reply_cmd(tx, format!("Set the time to {day_time}"));
        } else if sub.eq_ignore_ascii_case("add") {
            let Some(delta) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
                reply_cmd(tx, "Usage: /time add <ticks>".into());
                return;
            };
            let (game_time, day_time) = state.add_day_time(delta).await;
            state
                .broadcast_except(
                    &rad_core::packet::play::SetTime::overworld(game_time, day_time, 1.0),
                    None,
                )
                .await;
            reply_cmd(tx, format!("Set the time to {day_time}"));
        } else {
            reply_cmd(tx, "Usage: /time <set|add|query> ...".into());
        }
    } else if cmd.eq_ignore_ascii_case("reload") {
        if perm_level < 4 {
            reply_cmd(tx, "Permission denied.".into());
            return;
        }
        let reloaded = crate::radon::ServerRadonManager::load_dir(std::path::Path::new("radons"));
        *state.radons.write().await = reloaded;
        let loaded_recipes = {
            let world = state.world.read().await;
            world
                .world_dir()
                .map(crate::gameplay::inventory::load_custom_recipes_from_dir)
                .unwrap_or_default()
        };
        let rcount = loaded_recipes.len();
        *state.recipes.write().await = loaded_recipes;
        reply_cmd(
            tx,
            format!("Reloaded server radons and {rcount} custom recipe(s)."),
        );
    } else if cmd.eq_ignore_ascii_case("stats") {
        let Some(sender_tx) = tx else {
            reply_cmd(tx, "Stats are only available to players.".into());
            return;
        };
        let uuid = {
            let players = state.players.read().await;
            players
                .values()
                .find(|p| p.tx.same_channel(sender_tx))
                .map(|p| p.uuid)
        };
        if let Some(uuid) = uuid {
            let st = state
                .player_stats
                .read()
                .await
                .get(&uuid)
                .copied()
                .unwrap_or_default();
            send_chat_to(
                sender_tx,
                format!(
                    "Stats: blocks_broken={}, blocks_placed={}, mobs_killed={}",
                    st.blocks_broken, st.blocks_placed, st.mobs_killed
                ),
            );
        }
    } else if cmd.eq_ignore_ascii_case("dimension")
        || cmd.to_ascii_lowercase().starts_with("dimension ")
    {
        let Some(sender_tx) = tx else {
            reply_cmd(tx, "Dimension switch is only available to players.".into());
            return;
        };
        let dim_arg = cmd.split_once(' ').map_or("", |(_, rest)| rest).trim();
        let Some(dim) = crate::world::worldgen::DimensionId::parse(dim_arg) else {
            send_chat_to(sender_tx, "Usage: /dimension <overworld|nether|end>".into());
            return;
        };
        let sender_eid = {
            let players = state.players.read().await;
            players
                .iter()
                .find(|(_, p)| p.tx.same_channel(sender_tx))
                .map(|(&eid, _)| eid)
        };
        let Some(eid) = sender_eid else {
            return;
        };
        state.switch_player_dimension(eid, dim).await;
        send_chat_to(
            sender_tx,
            format!("Switched dimension to {}.", dim.resource_name()),
        );
    } else if (cmd.eq_ignore_ascii_case("lobby") || cmd.eq_ignore_ascii_case("hub"))
        && let Some(ref target) = state.config.lobby_server
    {
        if let Some(sender_tx) = tx {
            let mut data = BytesMut::new();
            write_bungee_utf(&mut data, "Connect");
            write_bungee_utf(&mut data, target);
            if let Ok(bytes) = encode_to_bytes(&PlayClientboundPluginMessage {
                channel: "bungeecord:main".into(),
                data: data.to_vec(),
            }) {
                let _ = sender_tx.try_send(bytes);
            }
        }
    } else if cmd.eq_ignore_ascii_case("stop") {
        if perm_level < 4 {
            reply_cmd(tx, "You do not have permission to use /stop.".into());
            return;
        }
        let _ = std::fs::write(".radserver.stop", b"stop");
        reply_cmd(tx, "Stopping the server...".into());
    }
}

async fn resolve_target_entity_ids(
    state: &ServerState,
    sender_tx: Option<&OutboundSender>,
    selector: &str,
) -> Vec<i32> {
    let players = state.players.read().await;
    match selector {
        "@s" => sender_tx
            .and_then(|stx| players.values().find(|p| p.tx.same_channel(stx)))
            .map(|p| vec![p.entity_id])
            .unwrap_or_default(),
        "@a" => players.values().map(|p| p.entity_id).collect(),
        "@p" => {
            if let Some(stx) = sender_tx
                && let Some(sender) = players.values().find(|p| p.tx.same_channel(stx))
            {
                players
                    .values()
                    .min_by(|a, b| {
                        let da = (a.x - sender.x).powi(2)
                            + (a.y - sender.y).powi(2)
                            + (a.z - sender.z).powi(2);
                        let db = (b.x - sender.x).powi(2)
                            + (b.y - sender.y).powi(2)
                            + (b.z - sender.z).powi(2);
                        da.total_cmp(&db)
                    })
                    .map(|p| vec![p.entity_id])
                    .unwrap_or_default()
            } else {
                players
                    .values()
                    .next()
                    .map(|p| vec![p.entity_id])
                    .unwrap_or_default()
            }
        }
        name => players
            .values()
            .find(|p| p.username.eq_ignore_ascii_case(name))
            .map(|p| vec![p.entity_id])
            .unwrap_or_default(),
    }
}

async fn kick_online_player(state: &ServerState, target: &str, reason: &str) -> bool {
    let players = state.players.read().await;
    let mut kicked = false;
    for p in players.values() {
        if p.username.eq_ignore_ascii_case(target) {
            if let Ok(bytes) = encode_to_bytes(&PlayDisconnect {
                reason: NbtTag::String(reason.to_owned()),
            }) {
                let _ = p.tx.try_send(bytes);
            }
            p.tx.disconnect();
            kicked = true;
        }
    }
    kicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ServerConfig, ServerForwardingMode};
    use crate::registries::{Version26_3Registries, synthetic_26_3_test_provider};
    use rad_core::codec::DEFAULT_MAX_PACKET_SIZE;
    use rad_core::packet::play::CommandNodeKind;
    use rad_core::version_package::LoadedVersionPackage;
    use std::collections::HashSet;
    use std::sync::Arc;

    fn test_server_state() -> ServerState {
        let cfg = ServerConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            forwarding_mode: ServerForwardingMode::None,
            forwarding_secret: b"server-test-secret".to_vec(),
            motd: "RadServer Test".into(),
            max_players: 50,
            view_distance: 2,
            simulation_distance: 2,
            tps: 20,
            game_mode: 0,
            spawn_x: 0.5,
            spawn_y: 64.0,
            spawn_z: 0.5,
            brand: "RadServer".into(),
            welcome_message: None,
            lobby_server: None,
            max_packet_size: DEFAULT_MAX_PACKET_SIZE,
            login_timeout_secs: 10,
            forwarding_timeout_secs: 5,
            configuration_timeout_secs: 10,
            play_read_timeout_secs: 35,
            admins: HashSet::new(),
            max_mobs: 256,
            autosave_interval_secs: 300,
            world_type: "flat".into(),
            world_seed: 0,
            level_name: "world".into(),
            difficulty: 1,
            pvp: true,
            force_gamemode: false,
            white_list: None,
            enforce_whitelist: false,
            op_permission_level: 4,
            spawn_protection: 0,
            enable_status: true,
            hide_online_players: false,
            hardcore: false,
            player_idle_timeout: 0,
            max_world_size: 29_999_984,
        };
        let pkg = Arc::new(LoadedVersionPackage::minimal_synthetic_for_tests(&[
            ("minecraft:air", 0),
            ("minecraft:stone", 1),
            ("minecraft:diamond", 2),
            ("minecraft:diamond_sword", 3),
        ]));
        let registries = Arc::new(
            Version26_3Registries::from_provider(Arc::new(synthetic_26_3_test_provider()), true)
                .expect("synthetic 26.3 registries"),
        );
        ServerState::new_with_world_dir_and_registries(cfg, None, pkg, registries)
    }

    fn node_name(node: &CommandNode) -> Option<&str> {
        match &node.kind {
            CommandNodeKind::Root => None,
            CommandNodeKind::Literal { name }
            | CommandNodeKind::GreedyStringArg { name }
            | CommandNodeKind::WordStringArg { name } => Some(name.as_str()),
        }
    }

    fn root_literal_names(pkt: &DeclareCommands) -> Vec<String> {
        let root = &pkt.nodes[pkt.root_index.0 as usize];
        root.children
            .iter()
            .filter_map(|&idx| node_name(&pkt.nodes[idx.0 as usize]).map(str::to_owned))
            .collect()
    }

    fn child_names_of(pkt: &DeclareCommands, literal: &str) -> Vec<String> {
        let root = &pkt.nodes[pkt.root_index.0 as usize];
        let Some(&node_idx) = root
            .children
            .iter()
            .find(|&&idx| node_name(&pkt.nodes[idx.0 as usize]) == Some(literal))
        else {
            return Vec::new();
        };
        pkt.nodes[node_idx.0 as usize]
            .children
            .iter()
            .filter_map(|&idx| node_name(&pkt.nodes[idx.0 as usize]).map(str::to_owned))
            .collect()
    }

    #[test]
    fn declare_commands_includes_subcommands_suggestions_and_filters_by_permission() {
        let lvl0 = build_declare_commands_for_level(0);
        let cmds0 = root_literal_names(&lvl0);
        assert!(cmds0.contains(&"help".to_owned()));
        assert!(cmds0.contains(&"list".to_owned()));
        assert!(!cmds0.contains(&"gamemode".to_owned()));
        assert!(!cmds0.contains(&"kick".to_owned()));
        assert!(!cmds0.contains(&"op".to_owned()));

        let lvl2 = build_declare_commands_for_level(2);
        let cmds2 = root_literal_names(&lvl2);
        assert!(cmds2.contains(&"gamemode".to_owned()));
        assert!(cmds2.contains(&"time".to_owned()));
        assert!(cmds2.contains(&"give".to_owned()));
        assert!(!cmds2.contains(&"kick".to_owned()));
        assert!(!cmds2.contains(&"op".to_owned()));

        let lvl3 = build_declare_commands_for_level(3);
        let cmds3 = root_literal_names(&lvl3);
        assert!(cmds3.contains(&"kick".to_owned()));
        assert!(cmds3.contains(&"ban".to_owned()));
        assert!(cmds3.contains(&"whitelist".to_owned()));
        assert!(cmds3.contains(&"op".to_owned()));
        assert!(!cmds3.contains(&"stop".to_owned()));

        let lvl4 = build_declare_commands_for_level(4);
        let cmds4 = root_literal_names(&lvl4);
        assert!(cmds4.contains(&"stop".to_owned()));
        assert!(cmds4.contains(&"save-all".to_owned()));
        assert!(cmds4.contains(&"reload".to_owned()));

        let gm_children = child_names_of(&lvl4, "gamemode");
        assert!(gm_children.contains(&"survival".to_owned()));
        assert!(gm_children.contains(&"creative".to_owned()));
        assert!(gm_children.contains(&"adventure".to_owned()));
        assert!(gm_children.contains(&"spectator".to_owned()));

        let time_children = child_names_of(&lvl4, "time");
        assert!(time_children.contains(&"set".to_owned()));
        assert!(time_children.contains(&"add".to_owned()));
        assert!(time_children.contains(&"query".to_owned()));

        let dim_children = child_names_of(&lvl4, "dimension");
        assert!(dim_children.contains(&"overworld".to_owned()));
        assert!(dim_children.contains(&"nether".to_owned()));
        assert!(dim_children.contains(&"end".to_owned()));

        let wl_children = child_names_of(&lvl4, "whitelist");
        assert!(wl_children.contains(&"on".to_owned()));
        assert!(wl_children.contains(&"off".to_owned()));
        assert!(wl_children.contains(&"add".to_owned()));
        assert!(wl_children.contains(&"remove".to_owned()));

        assert!(
            lvl4.nodes
                .iter()
                .any(|n| n.suggestions_type.as_deref() == Some("minecraft:ask_server"))
        );
    }

    #[tokio::test]
    async fn command_suggestions_are_filtered_by_permissions_and_suggest_arguments() {
        let state = test_server_state();
        let uuid = Uuid::new_v3(&Uuid::NAMESPACE_OID, b"test-player");

        // Non-operator (level 0) should only see level-0 commands and no /give suggestions.
        let root_lvl0 = build_command_suggestions(
            &state,
            "Guest",
            Some(uuid),
            &CommandSuggestionsRequest {
                id: VarInt(1),
                text: "/".to_owned(),
            },
        )
        .await;
        let texts0: Vec<&str> = root_lvl0.matches.iter().map(|m| m.text.as_str()).collect();
        assert!(texts0.contains(&"help"));
        assert!(!texts0.contains(&"give"));
        assert!(!texts0.contains(&"op"));

        let denied_give = build_command_suggestions(
            &state,
            "Guest",
            Some(uuid),
            &CommandSuggestionsRequest {
                id: VarInt(2),
                text: "/give dia".to_owned(),
            },
        )
        .await;
        assert!(denied_give.matches.is_empty());

        // Promote to level 2 operator and verify /gamemode + /give item suggestions.
        state
            .access
            .write()
            .await
            .add_op(None, "Builder".to_owned(), 2);

        let gm_sug = build_command_suggestions(
            &state,
            "Builder",
            Some(uuid),
            &CommandSuggestionsRequest {
                id: VarInt(3),
                text: "/gamemode cr".to_owned(),
            },
        )
        .await;
        assert_eq!(gm_sug.start, VarInt(10));
        assert_eq!(gm_sug.length, VarInt(2));
        assert_eq!(gm_sug.matches.len(), 1);
        assert_eq!(gm_sug.matches[0].text, "creative");
        assert!(gm_sug.matches[0].tooltip.is_some());

        let give_item_sug = build_command_suggestions(
            &state,
            "Builder",
            Some(uuid),
            &CommandSuggestionsRequest {
                id: VarInt(4),
                text: "/give Builder dia".to_owned(),
            },
        )
        .await;
        let item_texts: Vec<&str> = give_item_sug
            .matches
            .iter()
            .map(|m| m.text.as_str())
            .collect();
        assert!(item_texts.contains(&"minecraft:diamond"));
        assert!(item_texts.contains(&"minecraft:diamond_sword"));
        assert!(!item_texts.contains(&"minecraft:stone"));
    }
}
