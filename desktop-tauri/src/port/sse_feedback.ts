import { SseBackup } from "./sse_backup";
import type { FeedbackEnv, FeedbackImage, FeedbackMine, FeedbackReceipt, FeedbackReplyReceipt, FeedbackRequest } from "./feedback";

export class SseFeedback extends SseBackup {
  feedbackEnv(locale?: string) {
    return this.get<FeedbackEnv>("/feedback/env" + (locale ? "?locale=" + encodeURIComponent(locale) : ""));
  }
  sendFeedback(req: FeedbackRequest) {
    return this.post0<FeedbackReceipt>("/feedback", req);
  }
  myFeedback() {
    return this.get<FeedbackMine>("/feedback/mine");
  }
  replyFeedback(receipt: string, body: string, images: FeedbackImage[] = []) {
    return this.post0<FeedbackReplyReceipt>("/feedback/" + encodeURIComponent(receipt) + "/reply", images.length > 0 ? { body, images } : { body });
  }
  withdrawFeedback(receipt: string) {
    return this.post("/feedback/" + encodeURIComponent(receipt) + "/withdraw", {});
  }
  announceFeedbackReply() {
    return this.post("/notifications/feedback-reply", {});
  }
  ackFeedbackPromotion(level: number) {
    return this.post("/feedback/promotion/ack", { level });
  }
  feedbackSeen(receipt: string, upTo: number) {
    return this.post("/feedback/" + encodeURIComponent(receipt) + "/seen", { upTo });
  }
}
