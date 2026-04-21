(() => {
  "use strict";

  const GAUGE_ARC_LENGTH = Math.PI * 80;
  const DISSENT_METRIC_ID = "dissent_axis";

  const state = {
    forumId: null,
    topic: null,
    participants: [],
    maxRounds: 0,
    status: "pending",
    latestSeq: 0,
    rounds: new Map(),         // round -> { round, stage, responded: Set, synthesisWords, scoresByMetric: Map }
    metrics: [],               // [{ id, name, scale, description, mandatory }]
    latestScores: new Map(),   // metric_id -> number
    convergenceHistory: [],    // [{ round, score }]
    convergenceLatest: null,
    latestSynthesis: null,     // { round, wordCount }
    selectedRound: null,
  };

  const el = {
    forumId: byId("forum-id"),
    forumStatus: byId("forum-status"),
    currentRound: byId("current-round"),
    topic: byId("topic"),
    participants: byId("participants"),
    roundsGrid: byId("rounds-grid"),
    metrics: byId("metrics"),
    gaugeFill: byId("gauge-fill"),
    gaugeValue: byId("gauge-value"),
    convergenceHistory: byId("convergence-history"),
    responseTabs: byId("response-tabs"),
    responseBody: byId("response-body"),
    synthesisInfo: byId("synthesis-info"),
    connection: byId("connection"),
  };

  function byId(id) { return document.getElementById(id); }

  function ensureRound(round, stage) {
    let r = state.rounds.get(round);
    if (!r) {
      r = {
        round,
        stage: stage || "",
        responded: new Set(),
        synthesisWords: null,
        scoresByMetric: new Map(),
      };
      state.rounds.set(round, r);
    } else if (stage) {
      r.stage = stage;
    }
    return r;
  }

  function applyState(snapshot) {
    state.forumId = snapshot.forum_id;
    state.topic = snapshot.topic;
    state.participants = snapshot.participants || [];
    state.maxRounds = snapshot.max_rounds || 0;
    state.status = snapshot.status || "pending";
    state.latestSeq = snapshot.latest_seq || 0;

    state.rounds.clear();
    for (const r of snapshot.rounds || []) {
      const entry = ensureRound(r.round, r.stage);
      for (const p of r.participants_responded || []) entry.responded.add(p);
      if (r.synthesis && typeof r.synthesis.word_count === "number") {
        entry.synthesisWords = r.synthesis.word_count;
      }
      if (r.metric_scores && Array.isArray(r.metric_scores.scores)) {
        for (const s of r.metric_scores.scores) {
          entry.scoresByMetric.set(s.metric_id, s.score);
          state.latestScores.set(s.metric_id, s.score);
        }
      }
      if (typeof r.convergence_score === "number") {
        state.convergenceHistory.push({ round: r.round, score: r.convergence_score });
      }
    }

    if (snapshot.classifier_metrics && Array.isArray(snapshot.classifier_metrics.metrics)) {
      state.metrics = snapshot.classifier_metrics.metrics;
    }
    if (typeof snapshot.convergence_score === "number") {
      state.convergenceLatest = snapshot.convergence_score;
    }

    // Latest synthesis + selected round default to the highest round we know about.
    const highest = maxRoundNumber();
    if (highest !== null) {
      state.selectedRound = highest;
      const entry = state.rounds.get(highest);
      if (entry && entry.synthesisWords !== null) {
        state.latestSynthesis = { round: highest, wordCount: entry.synthesisWords };
      }
    }
  }

  function applyEvent(ev) {
    if (typeof ev.seq === "number" && ev.seq > state.latestSeq) {
      state.latestSeq = ev.seq;
    }
    const p = ev.payload || {};
    switch (ev.type) {
      case "forum_started":
        state.topic = p.topic || state.topic;
        state.participants = p.participants || state.participants;
        state.maxRounds = p.max_rounds || state.maxRounds;
        if (!state.forumId) state.forumId = ev.forum_id;
        state.status = "in_progress";
        break;
      case "round_started": {
        const r = ensureRound(p.round, p.stage);
        r.stage = p.stage || r.stage;
        state.selectedRound = p.round;
        state.status = "in_progress";
        break;
      }
      case "participant_response": {
        const r = ensureRound(p.round);
        if (p.participant) r.responded.add(p.participant);
        break;
      }
      case "synthesis": {
        const r = ensureRound(p.round);
        if (typeof p.word_count === "number") r.synthesisWords = p.word_count;
        state.latestSynthesis = { round: p.round, wordCount: p.word_count ?? null };
        break;
      }
      case "classifier_metrics":
        if (Array.isArray(p.metrics)) state.metrics = p.metrics;
        break;
      case "metric_scores": {
        const r = ensureRound(p.round);
        for (const s of p.scores || []) {
          r.scoresByMetric.set(s.metric_id, s.score);
          state.latestScores.set(s.metric_id, s.score);
        }
        break;
      }
      case "convergence":
        if (typeof p.score === "number") {
          state.convergenceLatest = p.score;
          state.convergenceHistory.push({ round: p.round, score: p.score });
        }
        break;
      case "forum_complete":
        state.status = "completed";
        break;
      case "claims":
      case "alignment":
        // Rendered implicitly via participant/synthesis events; no separate UI yet.
        break;
      default:
        // Unknown type — ignore so older clients don't break on newer event kinds.
        break;
    }
  }

  function maxRoundNumber() {
    let max = null;
    for (const k of state.rounds.keys()) {
      if (max === null || k > max) max = k;
    }
    return max;
  }

  function sortedRounds() {
    return Array.from(state.rounds.values()).sort((a, b) => a.round - b.round);
  }

  /* ---------- rendering ---------- */

  function render() {
    renderHeader();
    renderTopic();
    renderRounds();
    renderMetrics();
    renderGauge();
    renderResponses();
    renderSynthesis();
  }

  function renderHeader() {
    el.forumId.textContent = state.forumId || "—";
    el.forumStatus.textContent = state.status.replace("_", " ");
    el.forumStatus.dataset.status = state.status;
    const current = maxRoundNumber() ?? 0;
    el.currentRound.textContent = `${current} / ${state.maxRounds || "?"}`;
  }

  function renderTopic() {
    el.topic.textContent = state.topic || "Waiting for forum…";
    el.participants.replaceChildren(
      ...state.participants.map(name => {
        const chip = document.createElement("span");
        chip.className = "participant-chip";
        chip.textContent = name;
        return chip;
      }),
    );
  }

  function renderRounds() {
    const rounds = sortedRounds();
    if (rounds.length === 0) {
      el.roundsGrid.replaceChildren(hint("Rounds populate as the forum runs."));
      return;
    }
    const currentRound = maxRoundNumber();
    el.roundsGrid.replaceChildren(...rounds.map(r => {
      const row = document.createElement("div");
      row.className = "round-row";
      if (r.round === currentRound && state.status !== "completed") {
        row.classList.add("current");
      }

      const badge = document.createElement("span");
      badge.className = "round-badge";
      badge.textContent = `Round ${r.round}`;

      const stage = document.createElement("span");
      stage.className = "round-stage";
      stage.textContent = r.stage || "—";

      const dots = document.createElement("div");
      dots.className = "round-participants";
      for (const name of state.participants) {
        const dot = document.createElement("span");
        dot.className = "participant-dot";
        dot.dataset.state = r.responded.has(name) ? "responded" : "pending";
        const marker = document.createElement("span");
        marker.className = "marker";
        dot.append(marker, document.createTextNode(name));
        dots.appendChild(dot);
      }
      if (r.synthesisWords !== null) {
        const synth = document.createElement("span");
        synth.className = "participant-dot";
        synth.dataset.state = "synthesis";
        const marker = document.createElement("span");
        marker.className = "marker";
        synth.append(marker, document.createTextNode(`synthesis · ${r.synthesisWords} words`));
        dots.appendChild(synth);
      }

      row.append(badge, stage, dots);
      row.addEventListener("click", () => {
        state.selectedRound = r.round;
        renderResponses();
      });
      return row;
    }));
  }

  function renderMetrics() {
    if (state.metrics.length === 0) {
      el.metrics.replaceChildren(hint("Metrics appear after the pre-round classifier runs."));
      return;
    }
    // Dissent Axis first, then scale-asc, then alphabetical — stable and puts dissent up top.
    const sorted = [...state.metrics].sort((a, b) => {
      if (a.id === DISSENT_METRIC_ID) return -1;
      if (b.id === DISSENT_METRIC_ID) return 1;
      return a.name.localeCompare(b.name);
    });
    el.metrics.replaceChildren(...sorted.map(m => {
      const row = document.createElement("div");
      row.className = "metric";
      if (m.id === DISSENT_METRIC_ID || m.mandatory) row.classList.add("dissent");

      const labelWrap = document.createElement("div");
      const label = document.createElement("span");
      label.className = "metric-label";
      label.textContent = m.name;
      labelWrap.appendChild(label);
      if (m.description) {
        const desc = document.createElement("span");
        desc.className = "metric-desc";
        desc.textContent = m.description;
        labelWrap.appendChild(desc);
      }

      const bar = document.createElement("div");
      bar.className = "metric-bar";
      const fill = document.createElement("div");
      fill.className = "metric-fill";
      bar.appendChild(fill);

      const scoreEl = document.createElement("span");
      scoreEl.className = "metric-score";
      const score = state.latestScores.get(m.id);
      if (typeof score === "number") {
        const scale = m.scale || 10;
        const pct = Math.max(0, Math.min(100, (score / scale) * 100));
        requestAnimationFrame(() => { fill.style.width = pct + "%"; });
        scoreEl.textContent = `${score.toFixed(1)} / ${scale}`;
      } else {
        scoreEl.textContent = `— / ${m.scale || 10}`;
        scoreEl.classList.add("empty");
      }

      row.append(labelWrap, bar, scoreEl);
      return row;
    }));
  }

  function renderGauge() {
    const score = state.convergenceLatest;
    if (typeof score === "number") {
      const offset = GAUGE_ARC_LENGTH * (1 - Math.max(0, Math.min(10, score)) / 10);
      el.gaugeFill.style.strokeDashoffset = offset.toFixed(2);
      el.gaugeFill.style.stroke = score >= 7 ? "var(--green)" : score >= 4 ? "var(--yellow)" : "var(--red)";
      el.gaugeValue.textContent = score.toFixed(1);
    } else {
      el.gaugeFill.style.strokeDashoffset = GAUGE_ARC_LENGTH.toFixed(2);
      el.gaugeFill.style.stroke = "var(--accent)";
      el.gaugeValue.textContent = "—";
    }

    if (state.convergenceHistory.length === 0) {
      el.convergenceHistory.replaceChildren();
      return;
    }
    el.convergenceHistory.replaceChildren(...state.convergenceHistory.map(h => {
      const chip = document.createElement("span");
      chip.className = "history-chip";
      chip.textContent = `R${h.round} · ${h.score.toFixed(1)}`;
      return chip;
    }));
  }

  function renderResponses() {
    const rounds = sortedRounds();
    if (rounds.length === 0) {
      el.responseTabs.replaceChildren();
      el.responseBody.replaceChildren(hint("Rounds appear here once the forum starts."));
      return;
    }
    const selected = state.selectedRound ?? rounds[rounds.length - 1].round;
    el.responseTabs.replaceChildren(...rounds.map(r => {
      const tab = document.createElement("button");
      tab.type = "button";
      tab.className = "response-tab";
      if (r.round === selected) tab.classList.add("active");
      tab.textContent = `Round ${r.round}`;
      tab.addEventListener("click", () => {
        state.selectedRound = r.round;
        renderResponses();
      });
      return tab;
    }));

    const round = state.rounds.get(selected);
    if (!round) {
      el.responseBody.replaceChildren(hint("Round data not loaded yet."));
      return;
    }
    const grid = document.createElement("div");
    grid.className = "response-grid";
    for (const name of state.participants) {
      const cell = document.createElement("div");
      cell.className = "response-cell";
      const responded = round.responded.has(name);
      if (responded) cell.classList.add("responded");
      const nm = document.createElement("span");
      nm.className = "name";
      nm.textContent = name;
      const st = document.createElement("span");
      st.className = "status";
      st.textContent = responded ? "responded" : "waiting…";
      cell.append(nm, st);
      grid.appendChild(cell);
    }
    el.responseBody.replaceChildren(grid);
  }

  function renderSynthesis() {
    const latest = state.latestSynthesis;
    if (!latest) {
      el.synthesisInfo.classList.remove("has-synthesis");
      el.synthesisInfo.replaceChildren(hint("No synthesis yet."));
      return;
    }
    el.synthesisInfo.classList.add("has-synthesis");
    const badge = document.createElement("div");
    badge.className = "synthesis-round";
    badge.textContent = `Round ${latest.round}`;
    const words = document.createElement("div");
    words.className = "synthesis-words";
    words.textContent = latest.wordCount != null
      ? `Fire Keeper synthesis · ${latest.wordCount} words written to the session directory.`
      : "Fire Keeper synthesis written to the session directory.";
    el.synthesisInfo.replaceChildren(badge, words);
  }

  function hint(text) {
    const p = document.createElement("p");
    p.className = "hint";
    p.textContent = text;
    return p;
  }

  /* ---------- SSE ---------- */

  function setConnection(stateLabel) {
    el.connection.dataset.state = stateLabel;
    el.connection.textContent = {
      connecting: "connecting…",
      live: "live",
      ended: "stream ended",
      error: "connection error",
    }[stateLabel] || stateLabel;
  }

  function connect() {
    setConnection("connecting");
    const es = new EventSource("/api/events");

    es.addEventListener("init", ev => {
      try {
        const snapshot = JSON.parse(ev.data);
        applyState(snapshot);
        render();
        setConnection("live");
      } catch (err) {
        console.error("init parse error", err);
      }
    });

    es.addEventListener("update", ev => {
      try {
        const event = JSON.parse(ev.data);
        applyEvent(event);
        render();
        setConnection("live");
        if (event.type === "forum_complete") {
          es.close();
          setConnection("ended");
        }
      } catch (err) {
        console.error("update parse error", err);
      }
    });

    es.addEventListener("ping", () => { /* keep-alive only */ });

    es.onerror = () => {
      // EventSource auto-reconnects; surface the blip and let it retry.
      // If forum is already completed, treat as a clean close.
      if (state.status === "completed") {
        setConnection("ended");
        es.close();
      } else {
        setConnection("error");
      }
    };

    es.onopen = () => setConnection("live");
  }

  // Seed the UI from the snapshot before SSE lands so a completed-forum page
  // paints immediately even if the event stream is slow to arrive.
  fetch("/api/state", { cache: "no-store" })
    .then(r => (r.ok ? r.json() : null))
    .then(snapshot => {
      if (snapshot) {
        applyState(snapshot);
        render();
      } else {
        render();
      }
    })
    .catch(() => render())
    .finally(connect);
})();
