import { MockTheme } from "./mock_theme";
import type { BackupApplyResult, BackupCategory, BackupCategoryInfo, BackupFileCatalog, BackupFileExport, BackupPlan } from "./backup";

const CATEGORIES: BackupCategoryInfo[] = [
  { id: "settings", defaultOn: true, consent: false },
  { id: "extensions", defaultOn: true, consent: false },
  { id: "memory", defaultOn: true, consent: false },
  { id: "automation", defaultOn: true, consent: true },
  { id: "secrets", defaultOn: false, consent: false },
];

// The fixture answers the file export and import end to end, with no kernel
// behind it.
export class MockBackup extends MockTheme {
  async backupFileCatalog(): Promise<BackupFileCatalog> {
    return { categories: CATEGORIES, minPassphrase: 10 };
  }

  async exportBackupFile(_categories: BackupCategory[], _passphrase: string): Promise<BackupFileExport> {
    return { name: "reasonix-config-demo.rxcb", file: "UlhDQgAAAA==" };
  }

  async previewBackupFile(_file: string, _passphrase: string): Promise<BackupPlan> {
    return {
      planId: "demo", createdAt: new Date().toISOString(), platform: "darwin/arm64", samePlatform: true,
      categories: ["settings", "automation"],
      items: [
        { id: "provider:deepseek", category: "settings", kind: "provider", name: "deepseek", status: "same", summary: "openai https://api.deepseek.com", recommended: false },
        { id: "hook:PreToolUse#1", category: "automation", kind: "hook", name: "PreToolUse#1", status: "new", consent: "executes", summary: "npx prettier --check .", details: ["event: PreToolUse", "match: edit_file"], recommended: false },
        { id: "memory:docs/REASONIX.md", category: "memory", kind: "memory", name: "docs/REASONIX.md", status: "changed", summary: "REASONIX.md", content: "回答保持简洁。\n提交信息用英文。", recommended: true },
      ],
    };
  }

  async applyBackupFile(_planId: string, items: string[], _consented: string[]): Promise<BackupApplyResult> {
    return { applied: items };
  }
}