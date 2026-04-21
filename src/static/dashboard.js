(() => {
  "use strict";

  const GAUGE_ARC_LENGTH = Math.PI * 80;

  const state = {
    forumId: null,
    topic: null,
    participants: [],
    maxRounds: 0,
    status: "pending",
    rounds: new Map(),         // round -> { round, stage, responded: Set, synthesisWords, scoresByMetric: Map }
    metrics: [],               // [{ id, name, scale, description, mandatory }]
    convergenceHistory: [],    // [{ round, score }]
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
      if (state.selectedRound === null) state.selectedRound = round;
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

    state.rounds.clear();
    state.convergenceHistory = [];
    for (const r of snapshot.rounds || []) {
      const entry = ensureRound(r.round, r.stage);
      for (const p of r.participants_responded || []) entry.responded.add(p);
      if (r.synthesis && typeof r.synthesis.word_count === "number") {
        entry.synthesisWords = r.synthesis.word_count;
      }
      if (r.metric_scores && Array.isArray(r.metric_scores.scores)) {
        for (const s of r.metric_scores.scores) {
          entry.scoresByMetric.set(s.metric_id, s.score);
        }
      }
      if (typeof r.convergence_score === "number") {
        state.convergenceHistory.push({ round: r.round, score: r.convergence_score });
      }
    }

    if (snapshot.classifier_metrics && Array.isArray(snapshot.classifier_metrics.metrics)) {
      state.metrics = snapshot.classifier_metrics.metrics;
    }
    state.selectedRound = maxRoundNumber();
  }

  function applyEvent(ev) {
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
        ensureRound(p.round, p.stage);
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
        break;
      }
      case "classifier_metrics":
        if (Array.isArray(p.metrics)) state.metrics = p.metrics;
        break;
      case "metric_scores": {
        const r = ensureRound(p.round);
        for (const s of p.scores || []) {
          r.scoresByMetric.set(s.metric_id, s.score);
        }
        break;
      }
      case "convergence":
        if (typeof p.score === "number") {
          state.convergenceHistory.push({ round: p.round, score: p.score });
        }
        break;
      case "forum_complete":
        state.status = "completed";
        break;
      default:
        // claims / alignment are reflected via participant_response + synthesis.
        // Any unknown type is ignored so older clients don't break.
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

  function latestScores() {
    const rounds = sortedRounds();
    for (let i = rounds.length - 1; i >= 0; i--) {
      if (rounds[i].scoresByMetric.size > 0) return rounds[i].scoresByMetric;
    }
    return null;
  }

  function latestSynthesis() {
    const rounds = sortedRounds();
    for (let i = rounds.length - 1; i >= 0; i--) {
      if (rounds[i].synthesisWords !== null) {
        return { round: rounds[i].round, wordCount: rounds[i].synthesisWords };
      }
    }
    return null;
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
    el.currentRound.textContent = `${current} / ${state.maxRounds || "?"}`;
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
    // Mandatory metrics (Dissent Axis per plan-v2) first, then alphabetical.
    const sorted = [...state.metrics].sort((a, b) => {
      if (a.mandatory && !b.mandatory) return -1;
      if (b.mandatory && !a.mandatory) return 1;
      return a.name.localeCompare(b.name);
    });
    const scores = latestScores();
    el.metrics.replaceChildren(...sorted.map(m => {
      const row = document.createElement("div");
      row.className = "metric";
      if (m.mandatory) row.classList.add("dissent");

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
      const score = scores ? scores.get(m.id) : undefined;
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
    const history = state.convergenceHistory;
    const score = history.length ? history[history.length - 1].score : null;
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

    el.convergenceHistory.replaceChildren(...history.map(h => {
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
    const selected = state.selectedRound;
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
    const latest = latestSynthesis();
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
    }[stateLabel];
  }

  function connect() {
    setConnection("connecting");
    const es = new EventSource("/api/events");

    es.addEventListener("init", ev => {
      try {
        applyState(JSON.parse(ev.data));
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
      // If the forum already completed, the close is expected; otherwise
      // EventSource auto-reconnects — surface the blip and let it retry.
      if (state.status === "completed") {
        setConnection("ended");
        es.close();
      } else {
        setConnection("error");
      }
    };

    es.onopen = () => setConnection("live");
  }

  render();
  connect();
})();
