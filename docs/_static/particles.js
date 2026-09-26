(() => {
  "use strict";

  const physics = typeof module === "object" && module.exports
    ? require("./vendor/matter.min.js") : window.Matter;
  if (!physics) return;
  const { Bodies, Body, Composite, Engine } = physics;
  physics.Resolver._restingThresh = 0.001;
  const timestep = 1000 / 60;
  const randomBetween = (minimum, maximum) => minimum + Math.random() * (maximum - minimum);
  const formationKinds = ["grid", "geometry", "clusters", "wave"];
  const geometricShapes = ["ring", "square", "wave"];
  const smoothStep = value => {
    const bounded = Math.max(0, Math.min(1, value));
    return bounded * bounded * (3 - 2 * bounded);
  };
  const tileOpacity = (horizontal, articleWidth) => 1 - 0.5 * smoothStep((horizontal - articleWidth) / 48);

  class TileField {
    constructor(width, height) {
      this.engine = Engine.create({ gravity: { x: 0, y: 0, scale: 0 } });
      this.tiles = [];
      this.resize(width, height);
    }

    resize(width, height, maximumCount = 95) {
      this.width = Math.max(1, width);
      this.height = Math.max(1, height);
      this.maximumCount = maximumCount;
      Composite.clear(this.engine.world, false);
      Engine.clear(this.engine);
      this.tiles = [];
      this.formation = null;
      this.driftRemaining = randomBetween(15, 25);
      if (width < 60 || height < 60) return;
      const count = Math.min(maximumCount, Math.max(8, Math.round(width * height / 7800)));
      const columns = Math.ceil(Math.sqrt(count * width / height));
      const rows = Math.ceil(count / columns);
      for (let index = 0; index < count; index += 1) {
        const tileWidth = randomBetween(4, 9);
        const tileHeight = tileWidth;
        const horizontal = (index % columns + randomBetween(0.3, 0.7)) * width / columns;
        const vertical = (Math.floor(index / columns) + randomBetween(0.3, 0.7)) * height / rows;
        const body = Bodies.rectangle(horizontal, vertical, tileWidth, tileHeight, {
          chamfer: { radius: 1 },
          restitution: 1,
          friction: 0,
          frictionStatic: 0,
          frictionAir: 0,
          slop: 0.01,
        });
        const direction = randomBetween(0, Math.PI * 2);
        const speed = randomBetween(0.12, 0.28);
        const spin = randomBetween(0.0008, 0.0025) * (Math.random() < 0.5 ? -1 : 1);
        Body.setAngle(body, randomBetween(0, Math.PI * 2));
        Body.setVelocity(body, { x: Math.cos(direction) * speed, y: Math.sin(direction) * speed });
        Body.setAngularVelocity(body, spin);
        this.tiles.push({ body, width: tileWidth, height: tileHeight, speed, spin });
      }
      const wallOptions = { isStatic: true, restitution: 1, friction: 0, frictionStatic: 0 };
      Composite.add(this.engine.world, [
        ...this.tiles.map(tile => tile.body),
        Bodies.rectangle(width / 2, -25, width + 100, 50, wallOptions),
        Bodies.rectangle(width / 2, height + 25, width + 100, 50, wallOptions),
        Bodies.rectangle(-25, height / 2, 50, height + 100, wallOptions),
        Bodies.rectangle(width + 25, height / 2, 50, height + 100, wallOptions),
      ]);
    }

    startFormation(kind = formationKinds[Math.floor(Math.random() * formationKinds.length)],
      shape = geometricShapes[Math.floor(Math.random() * geometricShapes.length)]) {
      if (this.tiles.length < 8) return;
      const center = { x: this.width / 2, y: this.height / 2 };
      const targets = [];
      const addGrid = (count, origin, columns, spacing) => {
        const rows = Math.ceil(count / columns);
        for (let index = 0; index < count; index += 1) {
          targets.push({
            x: origin.x + (index % columns - (columns - 1) / 2) * spacing,
            y: origin.y + (Math.floor(index / columns) - (rows - 1) / 2) * spacing,
          });
        }
      };
      if (kind === "grid") {
        const columns = Math.ceil(Math.sqrt(this.tiles.length * this.width / this.height));
        const rows = Math.ceil(this.tiles.length / columns);
        const spacing = Math.min(26, (this.width - 24) / columns, (this.height - 24) / rows);
        addGrid(this.tiles.length, center, columns, spacing);
      } else if (kind === "clusters") {
        const count = Math.floor(this.tiles.length / 3);
        const horizontal = this.width >= this.height;
        for (let group = 0; group < 2; group += 1) {
          const members = Math.floor(count / 2) + (group === 0 ? count % 2 : 0);
          const origin = {
            x: center.x + (horizontal ? (group ? 1 : -1) * this.width * 0.23 : 0),
            y: center.y + (horizontal ? 0 : (group ? 1 : -1) * this.height * 0.23),
          };
          addGrid(members, origin, Math.ceil(Math.sqrt(members)), 18);
        }
      } else if (kind === "wave" || shape === "wave") {
        const span = this.width * 0.72;
        const count = Math.min(this.tiles.length, Math.max(3, Math.floor(span / 18)));
        const amplitude = Math.min(45, this.height * 0.12);
        for (let index = 0; index < count; index += 1) {
          const phase = index / (count - 1) * Math.PI * 2;
          targets.push({
            x: center.x - span / 2 + index / (count - 1) * span,
            y: center.y + Math.sin(phase) * amplitude,
            phase,
          });
        }
      } else {
        const radius = Math.min(this.width, this.height) * 0.28;
        const perimeter = radius * (shape === "square" ? 8 : Math.PI * 2);
        const count = Math.min(this.tiles.length, Math.max(4, Math.floor(perimeter / 18)));
        for (let index = 0; index < count; index += 1) {
          if (shape === "square") {
            const progress = index / count * 4;
            const side = Math.floor(progress);
            const offset = (progress - side) * radius * 2;
            targets.push({
              x: center.x + [-radius + offset, radius, radius - offset, -radius][side],
              y: center.y + [-radius, -radius + offset, radius, radius - offset][side],
            });
          } else {
            const angle = index / count * Math.PI * 2;
            targets.push({ x: center.x + Math.cos(angle) * radius, y: center.y + Math.sin(angle) * radius });
          }
        }
      }
      const available = new Set(this.tiles);
      const assignments = new Map();
      let farthest = 0;
      for (const target of targets) {
        let nearest = null;
        let nearestDistance = Infinity;
        for (const tile of available) {
          const distance = Math.hypot(tile.body.position.x - target.x, tile.body.position.y - target.y);
          if (distance < nearestDistance) {
            nearest = tile;
            nearestDistance = distance;
          }
        }
        available.delete(nearest);
        assignments.set(nearest, target);
        farthest = Math.max(farthest, nearestDistance);
      }
      this.formation = {
        kind, shape, assignments, age: 0,
        gather: 5 + farthest / 70,
        hold: randomBetween(2, 3),
        release: 2.5,
      };
    }

    advanceFormation() {
      if (!this.formation) {
        this.driftRemaining -= timestep / 1000;
        if (this.driftRemaining <= 0) this.startFormation();
        return;
      }
      const formation = this.formation;
      formation.age += timestep / 1000;
      if (formation.age >= formation.gather + formation.hold + formation.release) {
        this.formation = null;
        this.driftRemaining = randomBetween(15, 25);
      }
    }

    formationTarget(target) {
      if (this.formation.kind !== "wave") return target;
      const elapsed = Math.max(0, this.formation.age - this.formation.gather);
      const phase = target.phase - elapsed * 0.65;
      return {
        x: target.x,
        y: this.height / 2 + Math.sin(phase) * Math.min(45, this.height * 0.12),
      };
    }

    step(cursor) {
      this.advanceFormation();
      for (const tile of this.tiles) {
        const { body } = tile;
        let velocity = Body.getVelocity(body);
        const speed = Math.hypot(velocity.x, velocity.y);
        if (speed > 0.0001) {
          const relaxed = speed + (tile.speed - speed) * 0.012;
          velocity = { x: velocity.x * relaxed / speed, y: velocity.y * relaxed / speed };
        } else {
          velocity = { x: Math.cos(body.angle) * tile.speed, y: Math.sin(body.angle) * tile.speed };
        }
        let angularVelocity = Body.getAngularVelocity(body);
        angularVelocity = Math.max(-0.008, Math.min(0.008, angularVelocity + (tile.spin - angularVelocity) * 0.018));
        const formation = this.formation;
        const assignment = formation?.assignments.get(tile);
        if (assignment) {
          const target = this.formationTarget(assignment);
          const disrupted = cursor && (Math.hypot(body.position.x - cursor.x, body.position.y - cursor.y) < 110
            || Math.hypot(target.x - cursor.x, target.y - cursor.y) < 110);
          if (disrupted) {
            formation.assignments.delete(tile);
          } else {
            const releaseAge = formation.age - formation.gather - formation.hold;
            const strength = smoothStep(formation.age / 1.5) * (1 - smoothStep(releaseAge / formation.release));
            let targetVelocityX = (target.x - body.position.x) * 0.045;
            let targetVelocityY = (target.y - body.position.y) * 0.045;
            const targetSpeed = Math.hypot(targetVelocityX, targetVelocityY);
            if (targetSpeed > 1.6) {
              targetVelocityX *= 1.6 / targetSpeed;
              targetVelocityY *= 1.6 / targetSpeed;
            }
            velocity.x += (targetVelocityX - velocity.x) * 0.1 * strength;
            velocity.y += (targetVelocityY - velocity.y) * 0.1 * strength;
            const angleError = Math.atan2(Math.sin(body.angle * 4), Math.cos(body.angle * 4)) / 4;
            angularVelocity += (-angleError * 0.035 - angularVelocity) * 0.12 * strength;
          }
        }
        if (cursor) {
          const offsetX = body.position.x - cursor.x;
          const offsetY = body.position.y - cursor.y;
          const distance = Math.hypot(offsetX, offsetY);
          if (distance < 95) {
            const direction = distance > 0.01 ? Math.atan2(offsetY, offsetX) : body.angle;
            const impulse = 0.22 * (1 - distance / 95);
            velocity.x += Math.cos(direction) * impulse;
            velocity.y += Math.sin(direction) * impulse;
          }
        }
        const acceleratedSpeed = Math.hypot(velocity.x, velocity.y);
        if (acceleratedSpeed > 2.4) {
          velocity.x *= 2.4 / acceleratedSpeed;
          velocity.y *= 2.4 / acceleratedSpeed;
        }
        Body.setVelocity(body, velocity);
        Body.setAngularVelocity(body, angularVelocity);
      }
      Engine.update(this.engine, timestep);
    }
  }

  if (typeof module === "object" && module.exports) {
    module.exports = { TileField, tileOpacity };
    return;
  }

  const article = document.querySelector(".article-container");
  const toolbar = article?.querySelector(".content-icon-container");
  const main = article?.closest(".main");
  const contents = main?.querySelector(".toc-drawer");
  if (!article || !toolbar || !main) return;
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (!context) return;
  const layer = document.createElement("div");
  layer.className = "rdi-particle-layer";
  layer.setAttribute("aria-hidden", "true");
  layer.append(canvas);
  article.classList.add("rdi-particles");
  article.prepend(layer);

  const label = document.createElement("label");
  label.className = "rdi-motion-control";
  label.title = "Animate background";
  const toggle = document.createElement("input");
  toggle.type = "checkbox";
  toggle.setAttribute("aria-label", "Animate background");
  label.append(toggle, "Motion");
  toolbar.prepend(label);

  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");
  const field = new TileField(1, 1);
  let enabled = true;
  try { enabled = localStorage.getItem("rdi-background-motion") !== "off"; } catch {}
  let visible = true;
  let pageActive = true;
  let frame = 0;
  let lastTime = 0;
  let accumulator = 0;
  let pointer = null;
  let tileColor = "#737373";
  let articleWidth = 1;
  let expanded = false;

  function updatePalette() {
    const theme = document.body.dataset.theme;
    const dark = theme === "dark" || (theme !== "light" && darkScheme.matches);
    tileColor = dark ? "#b8b8b8" : "#737373";
    draw();
  }

  function draw() {
    context.clearRect(0, 0, field.width, field.height);
    for (const tile of field.tiles) {
      context.save();
      context.translate(tile.body.position.x, tile.body.position.y);
      context.rotate(tile.body.angle);
      context.fillStyle = tileColor;
      context.globalAlpha = tileOpacity(tile.body.position.x, articleWidth);
      context.beginPath();
      context.roundRect(-tile.width / 2, -tile.height / 2, tile.width, tile.height, 1);
      context.fill();
      context.restore();
    }
  }

  function animate(timestamp) {
    frame = 0;
    if (lastTime) accumulator += Math.min(timestamp - lastTime, 50);
    lastTime = timestamp;
    const bounds = canvas.getBoundingClientRect();
    const articleBounds = article.getBoundingClientRect();
    const cursor = pointer && pointer.x >= bounds.left && pointer.x <= bounds.right
      && pointer.y >= Math.max(bounds.top, articleBounds.top)
      && pointer.y <= Math.min(bounds.bottom, articleBounds.bottom)
      ? { x: pointer.x - bounds.left, y: pointer.y - bounds.top } : null;
    while (accumulator >= timestep) {
      field.step(cursor);
      accumulator -= timestep;
    }
    draw();
    frame = requestAnimationFrame(animate);
  }

  function syncPlayback() {
    cancelAnimationFrame(frame);
    frame = 0;
    lastTime = 0;
    accumulator = 0;
    toggle.checked = enabled && !reducedMotion.matches;
    toggle.disabled = reducedMotion.matches;
    layer.hidden = reducedMotion.matches;
    if (enabled && !reducedMotion.matches && !document.hidden && visible && pageActive) {
      frame = requestAnimationFrame(animate);
    }
  }

  function positionCanvas() {
    const bounds = article.getBoundingClientRect();
    const offset = Math.max(0, Math.min(-bounds.top, bounds.height - field.height));
    canvas.style.transform = `translateY(${offset}px)`;
  }

  function resize() {
    const articleBounds = article.getBoundingClientRect();
    const mainBounds = main.getBoundingClientRect();
    const contentsStyle = contents && getComputedStyle(contents);
    expanded = !!contentsStyle && contentsStyle.display !== "none"
      && contentsStyle.position === "static" && contents.getBoundingClientRect().width > 0;
    main.classList.toggle("rdi-particles", expanded);
    main.classList.toggle("rdi-particles-wide", expanded);
    article.classList.toggle("rdi-particles", !expanded);
    const host = expanded ? main : article;
    if (layer.parentElement !== host) host.prepend(layer);
    const hostBounds = host.getBoundingClientRect();
    articleWidth = articleBounds.width;
    const width = Math.round((expanded ? mainBounds.right : articleBounds.right) - articleBounds.left);
    const height = Math.round(Math.min(window.innerHeight, articleBounds.height));
    const maximumCount = expanded ? 125 : 95;
    layer.style.left = `${articleBounds.left - hostBounds.left}px`;
    layer.style.top = `${articleBounds.top - hostBounds.top}px`;
    layer.style.width = `${width}px`;
    layer.style.height = `${articleBounds.height}px`;
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    positionCanvas();
    if (field.width === width && field.height === height && field.maximumCount === maximumCount
      && canvas.width === Math.round(width * ratio)) {
      draw();
      return;
    }
    field.resize(width, height, maximumCount);
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
    canvas.style.width = `${width}px`;
    canvas.style.height = `${height}px`;
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    positionCanvas();
    draw();
    syncPlayback();
  }

  toggle.addEventListener("change", () => {
    enabled = toggle.checked;
    try { localStorage.setItem("rdi-background-motion", enabled ? "on" : "off"); } catch {}
    syncPlayback();
  });
  window.addEventListener("pointermove", event => {
    const blocked = event.target.closest("footer, .mobile-header, .overlay, .sidebar-drawer")
      || (!expanded && event.target.closest(".toc-drawer"));
    pointer = event.pointerType === "mouse" && !blocked
      ? { x: event.clientX, y: event.clientY } : null;
  }, { passive: true });
  document.documentElement.addEventListener("pointerleave", () => { pointer = null; });
  window.addEventListener("blur", () => { pointer = null; });
  window.addEventListener("resize", resize, { passive: true });
  window.addEventListener("scroll", positionCanvas, { passive: true, capture: true });
  window.addEventListener("pagehide", () => { pageActive = false; syncPlayback(); });
  window.addEventListener("pageshow", () => { pageActive = true; syncPlayback(); });
  document.addEventListener("visibilitychange", syncPlayback);
  reducedMotion.addEventListener("change", syncPlayback);
  darkScheme.addEventListener("change", updatePalette);
  new MutationObserver(updatePalette).observe(document.body, { attributes: true, attributeFilter: ["data-theme"] });
  const layoutObserver = new ResizeObserver(resize);
  layoutObserver.observe(article);
  layoutObserver.observe(main);
  new IntersectionObserver(entries => {
    visible = entries[0].isIntersecting;
    syncPlayback();
  }).observe(article);
  updatePalette();
  resize();
})();