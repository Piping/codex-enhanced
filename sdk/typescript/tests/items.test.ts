import { describe, expect, it } from "@jest/globals";

import type { ThreadItem } from "../src";

describe("ThreadItem types", () => {
  it("accepts collab tool call items", () => {
    const item: ThreadItem = {
      id: "item_1",
      type: "collab_tool_call",
      tool: "spawn_agent",
      sender_thread_id: "thread_main",
      receiver_thread_ids: ["thread_btw"],
      prompt: "continue",
      agents_states: {
        thread_btw: {
          status: "running",
          message: "working",
        },
      },
      status: "completed",
    };

    expect(item).toMatchObject({
      type: "collab_tool_call",
      receiver_thread_ids: ["thread_btw"],
    });
  });

  it("accepts web search items with an action payload", () => {
    const item: ThreadItem = {
      id: "item_2",
      type: "web_search",
      query: "codex",
      action: {
        type: "find_in_page",
        url: "https://example.com",
        pattern: "codex",
      },
    };

    expect(item).toMatchObject({
      type: "web_search",
      action: {
        type: "find_in_page",
        pattern: "codex",
      },
    });
  });
});
