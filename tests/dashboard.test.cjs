const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

function loadReducer() {
  // Exercise the shipped reducer without starting its DOM render or SSE client.
  const source = readFileSync(`${__dirname}/../src/static/dashboard.js`, "utf8");
  const startup = "  render();\n  connect();";
  assert.ok(source.includes(startup));
  const context = { document: { getElementById: () => ({}) } };
  vm.createContext(context);
  vm.runInContext(source.replace(startup, "globalThis.reducer = { state, applyState, applyEvent };"), context);
  return context.reducer;
}

test("live events and reconnect replay produce the same dashboard state", () => {
  const reducer = loadReducer();
  const events = [
    { seq: 1, forum_id: "test", type: "forum_started", payload: { topic: "A live topic", participants: ["alice", "bob"], max_rounds: 2 } },
    { seq: 2, type: "round_started", payload: { round: 1, stage: "proposal" } },
    { seq: 3, type: "participant_response", payload: { round: 1, participant: "alice" } },
    { seq: 4, type: "synthesis", payload: { round: 1, word_count: 20 } },
    { seq: 5, type: "convergence", payload: { round: 1, score: 8 } },
    { seq: 6, type: "forum_complete", payload: { rounds_used: 1 } },
  ];
  events.forEach(reducer.applyEvent);
  assert.equal(reducer.state.topic, "A live topic");
  assert.equal(reducer.state.forumId, "test");
  assert.equal(reducer.state.rounds.get(1).responded.has("alice"), true);
  assert.equal(reducer.state.status, "completed");
  events.forEach(reducer.applyEvent);
  assert.equal(reducer.state.convergenceHistory.length, 1);
  assert.equal(reducer.state.latestSeq, 6);
  assert.equal(reducer.state.status, "completed");
});

test("snapshot already accounts for its event-log prefix", () => {
  const reducer = loadReducer();
  reducer.applyState({ forum_id: "test", latest_seq: 5, topic: "A live topic", rounds: [{ round: 1, convergence_score: 8 }], status: "in_progress" });
  reducer.applyEvent({ seq: 5, type: "convergence", payload: { round: 1, score: 8 } });
  reducer.applyEvent({ seq: 6, type: "forum_complete", payload: { rounds_used: 1 } });
  assert.equal(reducer.state.convergenceHistory.length, 1);
  assert.equal(reducer.state.status, "completed");
});

test("failed forums remain failed when their log is replayed", () => {
  const reducer = loadReducer();
  reducer.applyEvent({ seq: 1, type: "forum_started", payload: { topic: "Failure", participants: ["alice"], max_rounds: 1 } });
  reducer.applyEvent({ seq: 2, type: "forum_failed", payload: { error: "Finalization failed" } });
  reducer.applyEvent({ seq: 1, type: "forum_started", payload: {} });
  assert.equal(reducer.state.status, "failed");
});
