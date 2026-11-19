import { SseIntake } from "./sse_intake";
import type { BackupApplyResult, BackupCategory, BackupFileCatalog, BackupFileExport, BackupPlan } from "./backup";

// The sealed config envelope as a file the person saves and opens again. Only
// the window may ask: the file can hold keys.
export class SseBackup extends SseIntake {
  backupFileCatalog() {
    return this.get<BackupFileCatalog>("/backups/file");
  }
  exportBackupFile(categories: BackupCategory[], passphrase: string) {
    return this.post0<BackupFileExport>("/backups/file/export", { categories, passphrase });
  }
  previewBackupFile(file: string, passphrase: string) {
    return this.post0<BackupPlan>("/backups/file/preview", { file, passphrase });
  }
  applyBackupFile(planId: string, items: string[], consented: string[]) {
    return this.post0<BackupApplyResult>("/backups/file/apply", { planId, items, consented });
  }
}