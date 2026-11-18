import type { BackupCategory } from "../../../port/backup";

// The backup catalogue's vocabulary, in one place because two blocks read it:
// the account's cloud backup (./data-backup.vue) and the file the person
// carries (./data-backup-file.vue). They differ in where the sealed envelope
// goes, never in what a category is called or what it warns about, so a second
// copy would be a second answer to the same question — and it was one, byte for
// byte, in both files.
//
// The React tree this frontend replaced keeps them the same way: one exported
// CATEGORY_LABEL in BackupRestore.tsx and one CATEGORY_NOTE in Backup.tsx.
export const CATEGORY_LABEL: Record<BackupCategory, string> = {
  settings: "data.models_and_interface",
  extensions: "data.skills_plugins_and_mcp",
  memory: "data.long_term_memory",
  automation: "data.hooks_and_status_line_command",
  secrets: "data.api_keys",
};

export const CATEGORY_NOTE: Record<BackupCategory, string> = {
  settings: "data.providers_default_model_presets_and",
  extensions: "data.plugins_are_recorded_by_source",
  memory: "data.user_level_instruction_files_and",
  automation: "data.commands_that_run_on_this",
  secrets: "data.off_by_default_when_ticked",
};
