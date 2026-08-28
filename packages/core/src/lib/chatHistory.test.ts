import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommandMock } = vi.hoisted(() => ({ invokeCommandMock: vi.fn() }));
vi.mock("../transport/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../transport/transport")>();
  return { ...actual, invokeCommand: invokeCommandMock };
});

import { listSessions, loadSession, saveSession, deleteSession, type Session, type SessionMeta } from "./chatHistory";

describe("chatHistory", () => {
  beforeEach(() => invokeCommandMock.mockReset());

  it("listSessions calls chat_history_list with no args and returns the metas as-is", async () => {
    const metas: SessionMeta[] = [
      { id: "newest", title: "Newest", createdAt: 100, updatedAt: 300 },
      { id: "oldest", title: "Oldest", createdAt: 1, updatedAt: 10 },
    ];
    invokeCommandMock.mockResolvedValue(metas);

    await expect(listSessions()).resolves.toEqual(metas);
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_history_list");
  });

  it("loadSession calls chat_history_load with the id and returns the full session", async () => {
    const session: Session = {
      id: "s1",
      title: "Scale the deployment",
      createdAt: 1,
      updatedAt: 2,
      contexts: ["prod-cluster"],
      skills: [],
      cliSessionId: null,
      messages: [{ role: "user", text: "hi" }],
    };
    invokeCommandMock.mockResolvedValue(session);

    await expect(loadSession("s1")).resolves.toEqual(session);
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_history_load", { id: "s1" });
  });

  it("saveSession calls chat_history_save with the session wrapped under a `session` key", async () => {
    const session: Session = {
      id: "s1",
      title: "Scale the deployment",
      createdAt: 1,
      updatedAt: 2,
      contexts: [],
      skills: [],
      cliSessionId: null,
      messages: [],
    };
    invokeCommandMock.mockResolvedValue(undefined);

    await saveSession(session);
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_history_save", { session });
  });

  it("deleteSession calls chat_history_delete with the id", async () => {
    invokeCommandMock.mockResolvedValue(undefined);

    await deleteSession("gone");
    expect(invokeCommandMock).toHaveBeenCalledWith("chat_history_delete", { id: "gone" });
  });
});
