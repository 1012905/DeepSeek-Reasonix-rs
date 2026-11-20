// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { h } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import "./testkit";
import { boot } from "../i18n";
import { JobsLive } from "./JobRunView.vue";
import { BackgroundNote } from "./cards/BackgroundJob.vue";
import type { JobEntry } from "../port/session";

beforeEach(() => boot());
const opened: VueWrapper[] = [];
afterEach(() => {
  while (opened.length > 0) opened.pop()?.unmount();
});

const job = (over: Partial<JobEntry> = {}): JobEntry =>
  ({ id: "j1", kind: "shell", label: "npm test", status: "running", startedAt: 1, ...over });

function draw(jobs: JobEntry[], open?: (entry: JobEntry) => void) {
  const wrapper = mount({
    render: () => h(JobsLive, { jobs, open, cancel: async () => {} }, { default: () => h(BackgroundNote, { id: "j1" }) }),
  });
  opened.push(wrapper);
  return wrapper;
}

describe("a running background job's note", () => {
  it("opens the job's output through the pane that provided the live list", async () => {
    const open = vi.fn();
    const wrapper = draw([job()], open);
    await wrapper.get('[data-action="job.view"]').trigger("click");
    expect(open).toHaveBeenCalledWith(expect.objectContaining({ id: "j1" }));
  });

  it("offers no view button when the pane cannot open one", () => {
    const wrapper = draw([job()]);
    expect(wrapper.find('[data-action="job.view"]').exists()).toBe(false);
  });

  it("says nothing once the job has ended", () => {
    const wrapper = draw([job({ status: "completed" })], vi.fn());
    expect(wrapper.find('[data-action="job.view"]').exists()).toBe(false);
    expect(wrapper.text()).toBe("");
  });
});
