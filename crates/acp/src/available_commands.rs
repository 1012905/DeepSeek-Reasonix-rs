//! ACP-044: available-command aggregation and slash-prompt resolution order
//! (`service.go:1246-1354`; `skill_hint.go:9-14`; `protocol.go:619-636`).
//!
//! The command/skill/prompt/action catalogs come from the live controller at
//! runtime; this module owns the pure aggregation rules — source order,
//! command-wins dedupe, default descriptions and hints, name sorting — and the
//! four-stage slash resolution chain.

use std::collections::BTreeMap;

use crate::update::{AvailableCommand, AvailableCommandInput};

/// A registered custom command (`service.go:1251-1268`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub arg_hint: String,
    pub hidden: bool,
}

/// A slash-exposed skill (`service.go:1269-1286`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlashSkill {
    pub slash_name: String,
    pub description: String,
    pub argument_hint: String,
}

/// An MCP host prompt (`service.go:1287-1303`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct McpPrompt {
    pub name: String,
    pub description: String,
    pub args_count: usize,
}

/// A handshake-declared extension action (`service.go:1306-1323`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtensionAction {
    /// Slash form, e.g. `<plugin>:<action>` with a leading `/` tolerated.
    pub slash: String,
    pub label: String,
    pub has_args: bool,
}

fn trimmed(value: &str) -> &str {
    value.trim()
}

/// Aggregates commands, skills, MCP prompts, and extension actions into the
/// advertised slash menu. Commands are inserted first and win any name clash;
/// the result is sorted by name (`service.go:1246-1329`).
pub fn available_commands(
    commands: &[SlashCommand],
    skills: &[SlashSkill],
    prompts: &[McpPrompt],
    actions: &[ExtensionAction],
) -> Vec<AvailableCommand> {
    let mut by_name: BTreeMap<String, AvailableCommand> = BTreeMap::new();

    for cmd in commands {
        let name = trimmed(&cmd.name).to_owned();
        if cmd.hidden || name.is_empty() {
            continue;
        }
        let description = match trimmed(&cmd.description) {
            "" => format!("Run the {name} command"),
            d => d.to_owned(),
        };
        let input = match trimmed(&cmd.arg_hint) {
            hint if !hint.is_empty() => Some(AvailableCommandInput {
                hint: hint.to_owned(),
            }),
            _ => None,
        };
        by_name.insert(
            name.clone(),
            AvailableCommand {
                name,
                description,
                input,
            },
        );
    }

    for skill in skills {
        let name = trimmed(&skill.slash_name).to_owned();
        if name.is_empty() || by_name.contains_key(&name) {
            continue;
        }
        let description = match trimmed(&skill.description) {
            "" => format!("Run the {name} skill"),
            d => d.to_owned(),
        };
        let hint = match trimmed(&skill.argument_hint) {
            hint if !hint.is_empty() => hint.to_owned(),
            _ => "instructions".to_owned(),
        };
        by_name.insert(
            name.clone(),
            AvailableCommand {
                name,
                description,
                input: Some(AvailableCommandInput { hint }),
            },
        );
    }

    for prompt in prompts {
        let name = trimmed(&prompt.name).to_owned();
        if name.is_empty() || by_name.contains_key(&name) {
            continue;
        }
        let description = match trimmed(&prompt.description) {
            "" => format!("Run the {name} MCP prompt"),
            d => d.to_owned(),
        };
        let input = (prompt.args_count > 0).then(|| AvailableCommandInput {
            hint: "arguments".to_owned(),
        });
        by_name.insert(
            name.clone(),
            AvailableCommand {
                name,
                description,
                input,
            },
        );
    }

    for action in actions {
        let name = trimmed(&action.slash).trim_start_matches('/').to_owned();
        if name.is_empty() || by_name.contains_key(&name) {
            continue;
        }
        let description = match trimmed(&action.label) {
            "" => format!("Run the {name} extension action"),
            d => d.to_owned(),
        };
        let input = action.has_args.then(|| AvailableCommandInput {
            hint: "arguments".to_owned(),
        });
        by_name.insert(
            name.clone(),
            AvailableCommand {
                name,
                description,
                input,
            },
        );
    }

    by_name.into_values().collect()
}

/// Resolution chain the runtime controller implements
/// (`service.go:1341-1351`). Each stage returns `None` to fall through to the
/// next.
pub trait SlashResolver {
    /// Registered custom command.
    fn custom_command(&self, line: &str) -> Option<String>;
    /// Skill by name; returns the expanded text and the invoked skill name.
    fn skill(&self, line: &str) -> Option<(String, String)>;
    /// MCP host prompt.
    fn mcp_prompt(&self, line: &str) -> Option<String>;
    /// Extension action against the handshake-declared catalog.
    fn extension_action(&self, line: &str) -> Option<String>;
}

/// Resolves a `/`-prefixed prompt through the fixed order: custom command,
/// skill, MCP prompt, extension action. A miss leaves the line untouched and
/// reports no invoked skill (`service.go:1332-1354`).
pub fn resolve_slash_prompt(text: &str, resolver: &dyn SlashResolver) -> (String, String) {
    let line = text.trim();
    if !line.starts_with('/') {
        return (text.to_owned(), String::new());
    }
    if let Some(sent) = resolver.custom_command(line) {
        return (sent, String::new());
    }
    if let Some((sent, skill)) = resolver.skill(line) {
        return (sent, skill);
    }
    if let Some(sent) = resolver.mcp_prompt(line) {
        return (sent, String::new());
    }
    if let Some(sent) = resolver.extension_action(line) {
        return (sent, String::new());
    }
    (text.to_owned(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct RecordingResolver {
        custom: Option<String>,
        skill: Option<(String, String)>,
        mcp: Option<String>,
        action: Option<String>,
    }

    impl SlashResolver for RecordingResolver {
        fn custom_command(&self, _line: &str) -> Option<String> {
            self.custom.clone()
        }
        fn skill(&self, _line: &str) -> Option<(String, String)> {
            self.skill.clone()
        }
        fn mcp_prompt(&self, _line: &str) -> Option<String> {
            self.mcp.clone()
        }
        fn extension_action(&self, _line: &str) -> Option<String> {
            self.action.clone()
        }
    }

    #[test]
    fn aggregates_four_sources_defaults_descriptions_and_sorts_by_name() {
        let commands = [SlashCommand {
            name: "ship".to_owned(),
            description: "  ".to_owned(),
            arg_hint: "branch".to_owned(),
            hidden: false,
        }];
        let skills = [SlashSkill {
            slash_name: "review".to_owned(),
            description: "".to_owned(),
            argument_hint: "".to_owned(),
        }];
        let prompts = [McpPrompt {
            name: "search".to_owned(),
            description: "".to_owned(),
            args_count: 1,
        }];
        let actions = [ExtensionAction {
            slash: "/alpha:act1".to_owned(),
            label: "Act one".to_owned(),
            has_args: true,
        }];

        let cmds = available_commands(&commands, &skills, &prompts, &actions);
        let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["alpha:act1", "review", "search", "ship"]);

        let ship = &cmds[3];
        assert_eq!(ship.description, "Run the ship command");
        assert_eq!(ship.input.as_ref().unwrap().hint, "branch");

        let review = &cmds[1];
        assert_eq!(review.description, "Run the review skill");
        assert_eq!(review.input.as_ref().unwrap().hint, "instructions");

        let search = &cmds[2];
        assert_eq!(search.description, "Run the search MCP prompt");
        assert_eq!(search.input.as_ref().unwrap().hint, "arguments");

        let action = &cmds[0];
        assert_eq!(action.description, "Act one");
        assert_eq!(action.input.as_ref().unwrap().hint, "arguments");
    }

    #[test]
    fn hidden_empty_and_duplicate_names_are_skipped_with_commands_winning() {
        let commands = [
            SlashCommand {
                name: "shared".to_owned(),
                description: "command wins".to_owned(),
                arg_hint: String::new(),
                hidden: false,
            },
            SlashCommand {
                name: "hidden".to_owned(),
                description: "x".to_owned(),
                arg_hint: String::new(),
                hidden: true,
            },
            SlashCommand {
                name: "  ".to_owned(),
                description: "x".to_owned(),
                arg_hint: String::new(),
                hidden: false,
            },
        ];
        let skills = [SlashSkill {
            slash_name: "shared".to_owned(),
            description: "skill loses".to_owned(),
            argument_hint: String::new(),
        }];
        let cmds = available_commands(&commands, &skills, &[], &[]);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].description, "command wins");
    }

    #[test]
    fn extension_action_slash_prefix_is_stripped_and_empty_skipped() {
        let actions = [
            ExtensionAction {
                slash: "/plain:act".to_owned(),
                label: String::new(),
                has_args: false,
            },
            ExtensionAction {
                slash: "  ".to_owned(),
                label: "x".to_owned(),
                has_args: false,
            },
        ];
        let cmds = available_commands(&[], &[], &[], &actions);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].name, "plain:act");
        assert_eq!(cmds[0].description, "Run the plain:act extension action");
        assert!(cmds[0].input.is_none());
    }

    #[test]
    fn resolution_order_is_command_skill_mcp_action_and_miss_passes_through() {
        let resolver = RecordingResolver {
            custom: Some("custom sent".to_owned()),
            skill: Some(("skill sent".to_owned(), "review".to_owned())),
            mcp: Some("mcp sent".to_owned()),
            action: Some("action sent".to_owned()),
        };
        assert_eq!(
            resolve_slash_prompt("/ship", &resolver),
            ("custom sent".to_owned(), String::new())
        );

        let skill_only = RecordingResolver {
            custom: None,
            skill: Some(("skill sent".to_owned(), "review".to_owned())),
            ..resolver.clone()
        };
        assert_eq!(
            resolve_slash_prompt("/review", &skill_only),
            ("skill sent".to_owned(), "review".to_owned())
        );

        let mcp_only = RecordingResolver {
            custom: None,
            skill: None,
            mcp: Some("mcp sent".to_owned()),
            ..resolver.clone()
        };
        assert_eq!(
            resolve_slash_prompt("/search", &mcp_only),
            ("mcp sent".to_owned(), String::new())
        );

        let action_only = RecordingResolver {
            custom: None,
            skill: None,
            mcp: None,
            action: Some("action sent".to_owned()),
        };
        assert_eq!(
            resolve_slash_prompt("/alpha:act1", &action_only),
            ("action sent".to_owned(), String::new())
        );

        let empty = RecordingResolver {
            custom: None,
            skill: None,
            mcp: None,
            action: None,
        };
        assert_eq!(
            resolve_slash_prompt("/alpha:other", &empty),
            ("/alpha:other".to_owned(), String::new())
        );
        assert_eq!(
            resolve_slash_prompt("plain text", &empty),
            ("plain text".to_owned(), String::new())
        );
    }
}
