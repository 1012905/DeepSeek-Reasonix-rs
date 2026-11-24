//! ACP-044: four-source command aggregation, command-wins dedupe, default
//! descriptions and hints, name sorting, and the four-stage slash resolution.

use reasonix_acp::available_commands::{
    ExtensionAction, McpPrompt, SlashCommand, SlashResolver, SlashSkill, available_commands,
    resolve_slash_prompt,
};

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
fn extension_action_strips_slash_prefix_and_empty_action_has_no_input() {
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

impl Clone for RecordingResolver {
    fn clone(&self) -> Self {
        Self {
            custom: self.custom.clone(),
            skill: self.skill.clone(),
            mcp: self.mcp.clone(),
            action: self.action.clone(),
        }
    }
}
