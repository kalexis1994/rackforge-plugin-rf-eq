/**
 * RF-EQ PLAY surface.
 *
 * The page builds itself from the parameter schema the host sends back, so it
 * has no private list of controls: adding one in the Rust contract makes it
 * appear here. Pages become groups; a float is a knob, a boolean a switch, an
 * enum a row of choices. Within a group, parameters that share the part of
 * their identifier before the dot — `peak1.frequency`, `peak1.gain`,
 * `peak1.q` — sit together in one strip, so a band reads as a band.
 *
 * Three things this surface has to get right to be usable on a stage rather
 * than in a screenshot.
 *
 * **It must not rebuild itself while a hand is on it.** The host sends a fresh
 * context whenever the session revision moves — which includes every write this
 * page makes — and replacing the DOM under a finger loses the gesture. The
 * panel is built once and patched thereafter.
 *
 * **It must not flood the host.** A Rack Slot edits its plugin through an
 * isolated instance: every `set_parameter` opens a plugin, loads state, applies
 * one value and saves it again. Writes are coalesced per parameter and sent one
 * at a time, at most sixteen times a second.
 *
 * **A value being edited belongs to the person editing it.** While a control is
 * held, values arriving from the host for that parameter are ignored, because
 * they are echoes of writes already in flight.
 */
(function () {
  "use strict";

  const PROTOCOL = "rackforge.plugin.web@1";
  const REQUEST_PREFIX = "rf-eq-";
  /// Milliseconds between flushes of the write queue.
  const WRITE_INTERVAL = 60;
  /// How long a status message stays before the connection line returns.
  const STATUS_LINGER = 4000;
  /// How long to wait for the host before giving up on a request.
  const REQUEST_TIMEOUT = 8000;

  /// What a strip is called, by the identifier prefix its parameters share.
  /// A prefix not listed here gets no strip: its controls sit in the group.
  const STRIP_NAMES = {
    hpf: "High-pass",
    low_shelf: "Low shelf",
    peak1: "Peak 1",
    peak2: "Peak 2",
    high_shelf: "High shelf",
  };

  const panelElement = document.getElementById("panel");
  const presetElement = document.getElementById("presets");
  const statusElement = document.getElementById("status");

  const state = {
    surface: "play",
    schema: null,
    values: new Map(),
    controls: new Map(),
    queue: new Map(),
    writtenAt: new Map(),
    held: new Set(),
    sounds: [],
    selectedSoundId: "",
    edited: false,
    built: false,
  };

  const pending = new Map();
  let nextRequest = 1;
  let writeEpoch = 0;
  let writeTimer = null;
  let writing = false;
  let lastWriteAt = 0;
  let refreshTimer = null;
  let statusTimer = null;

  /* --------------------------------------------------------------- bridge */

  function call(method, params) {
    const requestId = REQUEST_PREFIX + nextRequest++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(requestId);
        reject(new Error("RackForge did not answer in time"));
      }, REQUEST_TIMEOUT);
      pending.set(requestId, {
        resolve: (result) => {
          clearTimeout(timer);
          resolve(result);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      parent.postMessage(
        {
          protocol: PROTOCOL,
          kind: "request",
          request_id: requestId,
          method: method,
          params: params || {},
        },
        "*",
      );
    });
  }

  window.addEventListener("message", (event) => {
    const message = event.data;
    if (!message || message.protocol !== PROTOCOL) return;

    if (message.kind === "context") {
      state.surface = message.surface || "play";
      const instance = message.instance || {};
      state.selectedSoundId = instance.selected_sound_id || state.selectedSoundId;
      state.sounds = instance.sounds || state.sounds;
      renderPresetList();
      scheduleRefresh(120);
      return;
    }

    if (message.kind === "parameter_changed") {
      applyValues(
        [{ index: message.parameter_index, value: message.value }],
        writeEpoch,
      );
      return;
    }

    if (message.kind !== "response") return;
    const waiting = pending.get(message.request_id);
    if (!waiting) return;
    pending.delete(message.request_id);
    if (message.ok) waiting.resolve(message.result);
    else waiting.reject(new Error(message.error || "RackForge refused the request"));
  });

  /* --------------------------------------------------------------- input */

  function capture(element, pointerId) {
    try {
      element.setPointerCapture(pointerId);
    } catch (error) {
      /* the pointer is gone, or synthetic */
    }
  }

  function releaseCapture(element, pointerId) {
    try {
      element.releasePointerCapture(pointerId);
    } catch (error) {
      /* never captured, or already released */
    }
  }

  const gestures = new Set();

  function endGestures() {
    [...gestures].forEach((finish) => finish());
  }

  window.addEventListener("blur", endGestures);
  window.addEventListener("pagehide", endGestures);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) endGestures();
  });

  /* ---------------------------------------------------------------- status */

  function say(text, isError) {
    statusElement.textContent = text;
    statusElement.classList.toggle("error", Boolean(isError));
    clearTimeout(statusTimer);
    statusTimer = setTimeout(() => {
      statusElement.classList.remove("error");
      statusElement.textContent = connectionLine();
    }, STATUS_LINGER);
  }

  function connectionLine() {
    const sound = state.sounds.find((entry) => entry.id === state.selectedSoundId);
    const name = sound ? sound.name : "Custom setting";
    return name + (state.edited ? " · edited" : "") + " · " + state.surface + " surface";
  }

  function idle() {
    if (statusElement.classList.contains("error")) return;
    statusElement.textContent = connectionLine();
  }

  /* ----------------------------------------------------------------- model */

  function parametersOfPage(pageId) {
    return state.schema.parameters
      .filter((parameter) => parameter.page === pageId)
      .sort((left, right) => (left.order || 0) - (right.order || 0));
  }

  function valueOf(parameter) {
    const stored = state.values.get(parameter.index);
    if (Number.isFinite(stored)) return stored;
    const kind = parameter.kind;
    if (kind.type === "boolean") return kind.default ? 1 : 0;
    return kind.default;
  }

  /* ------------------------------------------------------------ write path */

  function write(parameter, value) {
    if (state.values.get(parameter.index) === value) return;
    writeEpoch += 1;
    state.values.set(parameter.index, value);
    state.queue.set(parameter.index, value);
    state.writtenAt.set(parameter.index, writeEpoch);
    if (!state.edited) {
      state.edited = true;
      idle();
    }
    scheduleFlush();
  }

  function scheduleFlush() {
    if (writeTimer !== null || writing) return;
    const due = Math.max(0, WRITE_INTERVAL - (Date.now() - lastWriteAt));
    writeTimer = setTimeout(flush, due);
  }

  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  async function flush() {
    writeTimer = null;
    if (writing || state.queue.size === 0) return;
    writing = true;
    try {
      while (state.queue.size > 0) {
        const idleFor = Date.now() - lastWriteAt;
        if (idleFor < WRITE_INTERVAL) await pause(WRITE_INTERVAL - idleFor);

        const [index, value] = state.queue.entries().next().value;
        state.queue.delete(index);
        lastWriteAt = Date.now();
        try {
          await call("plugin.set_parameter", { parameter_index: index, value: value });
        } catch (error) {
          const parameter = state.schema.parameters.find((candidate) => candidate.index === index);
          say((parameter ? parameter.name : "That control") + ": " + error.message, true);
        }
      }
    } finally {
      writing = false;
      if (state.queue.size > 0) scheduleFlush();
    }
  }

  /* ------------------------------------------------------------- rendering */

  function formatValue(parameter, value) {
    const kind = parameter.kind;
    if (kind.type === "float") {
      const unit = kind.unit ? " " + kind.unit : "";
      const span = kind.maximum - kind.minimum;
      const digits = span > 100 ? 0 : span > 4 ? 1 : 2;
      return value.toFixed(digits) + unit;
    }
    if (kind.type === "enum") {
      const choice = kind.choices.find((entry) => entry.value === Math.round(value));
      return choice ? choice.name : String(value);
    }
    if (kind.type === "boolean") return value >= 0.5 ? "on" : "off";
    if (kind.type === "integer") {
      return String(Math.round(value)) + (kind.unit ? " " + kind.unit : "");
    }
    return String(value);
  }

  /** Position 0..1 of a value on its knob, honouring a logarithmic taper. */
  function normalise(kind, value) {
    if (kind.taper === "logarithmic" && kind.minimum > 0) {
      return Math.log(value / kind.minimum) / Math.log(kind.maximum / kind.minimum);
    }
    return (value - kind.minimum) / (kind.maximum - kind.minimum);
  }

  function denormalise(kind, position) {
    const clamped = Math.min(1, Math.max(0, position));
    if (kind.taper === "logarithmic" && kind.minimum > 0) {
      return kind.minimum * Math.pow(kind.maximum / kind.minimum, clamped);
    }
    return kind.minimum + clamped * (kind.maximum - kind.minimum);
  }

  /** The label a knob shows: the parameter's name without the strip's. */
  function shortName(parameter) {
    const prefix = parameter.id.split(".")[0];
    const strip = STRIP_NAMES[prefix];
    if (!strip) return parameter.name;
    const bare = parameter.name.replace(/^peak \d\s+/i, "");
    return bare === parameter.name ? parameter.name.replace(/^(low|high|hpf)\s+/i, "") : bare;
  }

  function knobControl(parameter) {
    const kind = parameter.kind;
    const wrapper = document.createElement("div");
    wrapper.className = "knob";

    const size = 52;
    const radius = 20;
    const centre = size / 2;
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 " + size + " " + size);
    svg.setAttribute("width", String(size));
    svg.setAttribute("height", String(size));
    svg.setAttribute("role", "slider");
    svg.setAttribute("tabindex", "0");
    svg.setAttribute("aria-label", parameter.name);
    svg.setAttribute("aria-valuemin", String(kind.minimum));
    svg.setAttribute("aria-valuemax", String(kind.maximum));

    const track = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    track.setAttribute("cx", String(centre));
    track.setAttribute("cy", String(centre));
    track.setAttribute("r", String(radius));
    track.setAttribute("class", "knob-body");
    svg.appendChild(track);

    const sweep = document.createElementNS("http://www.w3.org/2000/svg", "path");
    sweep.setAttribute("class", "knob-sweep");
    sweep.setAttribute("fill", "none");
    svg.appendChild(sweep);

    const pointer = document.createElementNS("http://www.w3.org/2000/svg", "line");
    pointer.setAttribute("class", "knob-pointer");
    svg.appendChild(pointer);

    const label = document.createElement("div");
    label.className = "label";
    label.textContent = shortName(parameter);
    const reading = document.createElement("div");
    reading.className = "reading";

    const angleOf = (value) => (-225 + 270 * normalise(kind, value)) * (Math.PI / 180);
    const pointAt = (angle, distance) => [
      centre + Math.cos(angle) * distance,
      centre + Math.sin(angle) * distance,
    ];

    // A gain knob sweeps from its centre, a frequency knob from its start,
    // so a band at zero reads as a pointer straight up with no arc.
    const bipolar = kind.minimum < 0 && kind.maximum > 0;
    const origin = bipolar ? 0 : kind.minimum;

    function paint(value) {
      const angle = angleOf(value);
      const [tipX, tipY] = pointAt(angle, radius * 0.78);
      pointer.setAttribute("x1", String(centre));
      pointer.setAttribute("y1", String(centre));
      pointer.setAttribute("x2", String(tipX));
      pointer.setAttribute("y2", String(tipY));

      // The arc always runs clockwise, so below the origin it starts at
      // the pointer and ends at the origin.
      const start = angleOf(origin);
      const from = value < origin ? angle : start;
      const to = value < origin ? start : angle;
      const [fromX, fromY] = pointAt(from, radius * 0.92);
      const [toX, toY] = pointAt(to, radius * 0.92);
      const degrees = Math.abs(270 * (normalise(kind, value) - normalise(kind, origin)));
      const large = degrees > 180 ? 1 : 0;
      sweep.setAttribute(
        "d",
        "M " + fromX + " " + fromY +
          " A " + radius * 0.92 + " " + radius * 0.92 + " 0 " + large + " 1 " + toX + " " + toY,
      );

      reading.textContent = formatValue(parameter, value);
      svg.setAttribute("aria-valuenow", String(value));
      svg.setAttribute("aria-valuetext", reading.textContent);
    }

    function quantise(value) {
      const step = kind.step || 0.001;
      const stepped = Math.round(value / step) * step;
      return Math.min(kind.maximum, Math.max(kind.minimum, stepped));
    }

    let dragging = false;
    let pointerId = null;
    let startY = 0;
    let startPosition = 0;

    svg.addEventListener("pointerdown", (event) => {
      dragging = true;
      pointerId = event.pointerId;
      startY = event.clientY;
      startPosition = normalise(kind, valueOf(parameter));
      state.held.add(parameter.index);
      capture(svg, pointerId);
      svg.classList.add("held");
      gestures.add(finish);
      event.preventDefault();
    });

    svg.addEventListener("pointermove", (event) => {
      if (!dragging) return;
      // Two hundred pixels covers the range; shift slows it to a crawl for the
      // settings that need it.
      const scale = event.shiftKey ? 800 : 200;
      const position = startPosition + (startY - event.clientY) / scale;
      const value = quantise(denormalise(kind, position));
      paint(value);
      write(parameter, value);
    });

    function finish() {
      if (!dragging) return;
      dragging = false;
      gestures.delete(finish);
      state.held.delete(parameter.index);
      svg.classList.remove("held");
      releaseCapture(svg, pointerId);
      scheduleRefresh();
    }

    svg.addEventListener("pointerup", finish);
    svg.addEventListener("pointercancel", finish);
    svg.addEventListener("lostpointercapture", finish);

    svg.addEventListener("dblclick", () => {
      paint(kind.default);
      write(parameter, kind.default);
    });

    svg.addEventListener("keydown", (event) => {
      const stepPosition = 1 / (event.shiftKey ? 200 : 20);
      let position = normalise(kind, valueOf(parameter));
      if (event.key === "ArrowUp" || event.key === "ArrowRight") position += stepPosition;
      else if (event.key === "ArrowDown" || event.key === "ArrowLeft") position -= stepPosition;
      else if (event.key === "Home") position = 0;
      else if (event.key === "End") position = 1;
      else return;
      event.preventDefault();
      const value = quantise(denormalise(kind, position));
      paint(value);
      write(parameter, value);
    });

    paint(valueOf(parameter));
    wrapper.append(svg, label, reading);
    state.controls.set(parameter.index, { apply: paint });
    return wrapper;
  }

  function toggleControl(parameter) {
    const wrapper = document.createElement("div");
    wrapper.className = "knob";
    const button = document.createElement("button");
    button.type = "button";
    button.className = "toggle";
    button.textContent = parameter.name;
    button.setAttribute("aria-label", parameter.name);
    button.addEventListener("click", () => {
      const next = valueOf(parameter) >= 0.5 ? 0 : 1;
      paint(next);
      write(parameter, next);
    });
    const reading = document.createElement("div");
    reading.className = "reading";

    function paint(value) {
      const on = value >= 0.5;
      button.setAttribute("aria-pressed", String(on));
      reading.textContent = on ? "on" : "off";
    }

    paint(valueOf(parameter));
    wrapper.append(button, reading);
    state.controls.set(parameter.index, { apply: paint });
    return wrapper;
  }

  function choiceControl(parameter) {
    const wrapper = document.createElement("div");
    wrapper.className = "knob wide";
    const label = document.createElement("div");
    label.className = "label";
    label.textContent = parameter.name;
    const row = document.createElement("div");
    row.className = "choice";
    row.setAttribute("role", "radiogroup");
    row.setAttribute("aria-label", parameter.name);

    const buttons = parameter.kind.choices.map((choice) => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = choice.name;
      button.setAttribute("role", "radio");
      button.addEventListener("click", () => {
        paint(choice.value);
        write(parameter, choice.value);
      });
      row.appendChild(button);
      return button;
    });

    function paint(value) {
      const selected = Math.round(value);
      buttons.forEach((button, index) => {
        const isSelected = parameter.kind.choices[index].value === selected;
        button.setAttribute("aria-checked", String(isSelected));
        button.setAttribute("aria-pressed", String(isSelected));
      });
    }

    paint(valueOf(parameter));
    wrapper.append(label, row);
    state.controls.set(parameter.index, { apply: paint });
    return wrapper;
  }

  function control(parameter) {
    switch (parameter.kind.type) {
      case "enum":
        return choiceControl(parameter);
      case "boolean":
        return toggleControl(parameter);
      default:
        return knobControl(parameter);
    }
  }

  /** A band's controls in one strip, named after the band. */
  function stripCard(name, parameters) {
    const strip = document.createElement("div");
    strip.className = "strip";
    strip.setAttribute("role", "group");
    strip.setAttribute("aria-label", name);
    const title = document.createElement("div");
    title.className = "strip-name";
    title.textContent = name;
    const knobs = document.createElement("div");
    knobs.className = "knobs";
    parameters.forEach((parameter) => knobs.appendChild(control(parameter)));
    strip.append(title, knobs);
    return strip;
  }

  function groupCard(page) {
    const card = document.createElement("section");
    card.className = "group";
    card.dataset.page = page.id;
    card.setAttribute("aria-label", page.name);
    const name = document.createElement("div");
    name.className = "group-name";
    name.textContent = page.name;
    const knobs = document.createElement("div");
    knobs.className = "knobs";

    // Consecutive parameters sharing a named prefix become one strip; the
    // rest stand on their own, in schema order.
    const parameters = parametersOfPage(page.id);
    let position = 0;
    while (position < parameters.length) {
      const prefix = parameters[position].id.split(".")[0];
      const stripName = STRIP_NAMES[prefix];
      if (!stripName) {
        knobs.appendChild(control(parameters[position]));
        position += 1;
        continue;
      }
      const members = [];
      while (position < parameters.length && parameters[position].id.split(".")[0] === prefix) {
        members.push(parameters[position]);
        position += 1;
      }
      knobs.appendChild(stripCard(stripName, members));
    }
    card.append(name, knobs);
    return card;
  }

  /* ------------------------------------------------------------- presets */

  function renderPresetList() {
    const current = presetElement.value;
    presetElement.textContent = "";
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = state.sounds.length ? "Factory settings…" : "No factory settings";
    presetElement.appendChild(placeholder);
    state.sounds.forEach((sound) => {
      const option = document.createElement("option");
      option.value = sound.id;
      option.textContent = sound.name;
      option.title = sound.detail || "";
      presetElement.appendChild(option);
    });
    presetElement.value = state.selectedSoundId || current || "";
  }

  presetElement.addEventListener("change", () => {
    const soundId = presetElement.value;
    if (!soundId) return;
    call("plugin.select_sound", { sound_id: soundId })
      .then(() => {
        state.selectedSoundId = soundId;
        state.edited = false;
        say("Loaded " + presetElement.selectedOptions[0].textContent, false);
        return refresh();
      })
      .catch((error) => say("Could not load that setting: " + error.message, true));
  });

  /* --------------------------------------------------------------- wiring */

  function applyValues(values, readEpoch) {
    (values || []).forEach((entry) => {
      if (!Number.isInteger(entry.index) || !Number.isFinite(entry.value)) return;
      if (state.held.has(entry.index)) return;
      if ((state.writtenAt.get(entry.index) || 0) > readEpoch) return;
      state.values.set(entry.index, entry.value);
      const widget = state.controls.get(entry.index);
      if (widget) widget.apply(entry.value);
    });
  }

  function scheduleRefresh(delay) {
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(refresh, delay === undefined ? 150 : delay);
  }

  async function refresh() {
    if (state.queue.size > 0 || writing || state.held.size > 0) {
      scheduleRefresh();
      return;
    }
    const readEpoch = writeEpoch;
    try {
      const parameters = await call("plugin.parameters");
      const schemaChanged =
        !state.schema ||
        state.schema.parameters.length !== parameters.schema.parameters.length;
      state.schema = parameters.schema;
      if (!state.built || schemaChanged) {
        build();
      }
      applyValues(parameters.values, readEpoch);
      idle();
    } catch (error) {
      say("RackForge did not answer: " + error.message, true);
    }
  }

  function build() {
    state.controls.clear();
    state.writtenAt.clear();
    panelElement.textContent = "";
    [...state.schema.pages]
      .sort((left, right) => (left.order || 0) - (right.order || 0))
      .forEach((page) => panelElement.appendChild(groupCard(page)));
    state.built = true;
  }

  parent.postMessage({ protocol: PROTOCOL, kind: "ready" }, "*");
})();
