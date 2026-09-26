const assert = require("node:assert/strict");
const test = require("node:test");
const { TileField, tileOpacity } = require("../docs/_static/particles.js");
const { Body, Composite } = require("../docs/_static/vendor/matter.min.js");

test.beforeEach(context => {
  let seed = 1937;
  context.mock.method(Math, "random", () => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return seed / 4294967296;
  });
});

function isolatedTile() {
  const field = new TileField(600, 400);
  const tile = field.tiles[0];
  Composite.clear(field.engine.world, false);
  Composite.add(field.engine.world, tile.body);
  field.tiles = [tile];
  Body.setPosition(tile.body, { x: 300, y: 200 });
  return { field, tile };
}

test("density is bounded and tiles are small squares with slow motion", () => {
  for (const [width, height] of [[360, 740], [900, 900], [4000, 2000]]) {
    const field = new TileField(width, height);
    assert.equal(field.tiles.length, Math.min(95, Math.max(8, Math.round(width * height / 7800))));
    for (const tile of field.tiles) {
      assert.ok(tile.width >= 4 && tile.width <= 9);
      assert.equal(tile.height, tile.width);
      assert.ok(tile.speed >= 0.12 && tile.speed <= 0.28);
      assert.ok(Math.abs(tile.spin) >= 0.0008 && Math.abs(tile.spin) <= 0.0025);
    }
  }
});

test("expanded fields raise the cap while article-only fields retain their density", () => {
  const field = new TileField(343, 844);
  assert.equal(field.tiles.length, 37);
  field.resize(1100, 900, 125);
  assert.equal(field.tiles.length, 125);
  field.resize(4000, 900, 125);
  assert.equal(field.tiles.length, 125);
  field.resize(736, 900);
  assert.equal(field.tiles.length, 85);
  field.resize(343, 844);
  assert.equal(field.tiles.length, 37);
});

test("right-side opacity smoothly fades to half without changing the article", () => {
  assert.equal(tileOpacity(0, 736), 1);
  assert.equal(tileOpacity(736, 736), 1);
  assert.equal(tileOpacity(760, 736), 0.75);
  assert.equal(tileOpacity(784, 736), 0.5);
  assert.equal(tileOpacity(1100, 736), 0.5);
});

test("expanded fields allow tiles to cross the article edge in both directions", () => {
  const field = new TileField(1100, 900);
  field.resize(1100, 900, 125);
  const tile = field.tiles[0];
  Composite.clear(field.engine.world, false);
  Composite.add(field.engine.world, tile.body);
  field.tiles = [tile];
  Body.setPosition(tile.body, { x: 730, y: 450 });
  Body.setVelocity(tile.body, { x: tile.speed, y: 0 });
  for (let tick = 0; tick < 120; tick += 1) field.step(null);
  assert.ok(tile.body.position.x > 736);
  Body.setVelocity(tile.body, { x: -tile.speed, y: 0 });
  for (let tick = 0; tick < 240; tick += 1) field.step(null);
  assert.ok(tile.body.position.x < 736);
});

test("tiles drift and spin without gravity", () => {
  const { field, tile } = isolatedTile();
  const start = { ...tile.body.position };
  const angle = tile.body.angle;
  for (let tick = 0; tick < 60; tick += 1) field.step(null);
  const distance = Math.hypot(tile.body.position.x - start.x, tile.body.position.y - start.y);
  assert.ok(distance >= 7 && distance <= 18);
  assert.ok(Math.abs(tile.body.angle - angle) > 0.04);
  assert.equal(field.engine.gravity.scale, 0);
});

test("equal-mass head-on contacts bounce apart", () => {
  const field = new TileField(600, 400);
  const [first, second] = field.tiles;
  Composite.clear(field.engine.world, false);
  field.tiles = [first, second];
  Composite.add(field.engine.world, field.tiles.map(tile => tile.body));
  for (const tile of field.tiles) {
    Body.setMass(tile.body, 1);
    Body.setAngle(tile.body, 0);
    Body.setAngularVelocity(tile.body, 0);
    tile.speed = 0.2;
    tile.spin = 0;
  }
  Body.setPosition(first.body, { x: 250, y: 200 });
  Body.setPosition(second.body, { x: 290, y: 200 });
  Body.setVelocity(first.body, { x: 0.2, y: 0 });
  Body.setVelocity(second.body, { x: -0.2, y: 0 });
  for (let tick = 0; tick < 160; tick += 1) field.step(null);
  assert.ok(first.body.velocity.x < -0.15);
  assert.ok(second.body.velocity.x > 0.15);
  assert.ok(first.body.bounds.max.x < second.body.bounds.min.x);
});

test("cursor repulsion briefly accelerates then relaxes back to drift", () => {
  const { field, tile } = isolatedTile();
  Body.setVelocity(tile.body, { x: -tile.speed, y: 0 });
  const cursor = { x: 275, y: 200 };
  for (let tick = 0; tick < 20; tick += 1) field.step(cursor);
  assert.ok(tile.body.velocity.x > tile.speed * 2);
  assert.ok(tile.body.position.x > 300);
  assert.ok(Body.getSpeed(tile.body) <= 2.401);
  for (let tick = 0; tick < 600; tick += 1) field.step(null);
  assert.ok(Math.abs(Body.getSpeed(tile.body) - tile.speed) < 0.005);
});

test("a cursor exactly at a tile center does not produce invalid physics", () => {
  const { field, tile } = isolatedTile();
  field.step({ ...tile.body.position });
  assert.ok(Number.isFinite(tile.body.velocity.x));
  assert.ok(Number.isFinite(tile.body.velocity.y));
});

test("walls contain the tiles through collisions and cursor impulses", () => {
  const field = new TileField(360, 600);
  for (let tick = 0; tick < 1800; tick += 1) field.step({ x: 180, y: 300 });
  for (const { body } of field.tiles) {
    assert.ok(body.bounds.min.x >= -1 && body.bounds.max.x <= 361);
    assert.ok(body.bounds.min.y >= -1 && body.bounds.max.y <= 601);
  }
  field.resize(600, 300);
  assert.equal(Composite.allBodies(field.engine.world).length, field.tiles.length + 4);
  field.resize(20, 20);
  assert.equal(field.tiles.length, 0);
});

const patterns = [
  ["grid", "ring"], ["geometry", "ring"], ["geometry", "square"],
  ["geometry", "wave"], ["clusters", "ring"], ["wave", "wave"],
];

for (const [kind, shape] of patterns) {
  test(`${kind}/${shape} gathers, holds and releases on desktop, mobile and expanded fields`, () => {
    for (const [width, height, maximumCount] of [[736, 900, 95], [343, 740, 95], [1100, 900, 125]]) {
      const field = new TileField(width, height);
      field.resize(width, height, maximumCount);
      const initialPositions = field.tiles.map(tile => ({ ...tile.body.position }));
      field.startFormation(kind, shape);
      const formation = field.formation;
      assert.ok(formation.hold >= 2 && formation.hold <= 3);
      assert.deepEqual(field.tiles.map(tile => tile.body.position), initialPositions);
      if (kind === "clusters") assert.equal(formation.assignments.size, Math.floor(field.tiles.length / 3));
      if (kind === "grid") assert.equal(formation.assignments.size, field.tiles.length);
      const targets = [...formation.assignments.values()];
      for (let index = 0; index < targets.length; index += 1) {
        const target = targets[index];
        assert.ok(target.x >= 8 && target.x <= width - 8);
        assert.ok(target.y >= 8 && target.y <= height - 8);
        for (const other of targets.slice(index + 1)) {
          assert.ok(Math.hypot(target.x - other.x, target.y - other.y) >= 12);
        }
      }
      while (formation.age < formation.gather + formation.hold / 2) field.step(null);
      const distances = [...formation.assignments].map(([tile, assignment]) => {
        const target = field.formationTarget(assignment);
        return Math.hypot(tile.body.position.x - target.x, tile.body.position.y - target.y);
      });
      const meanDistance = distances.reduce((total, distance) => total + distance, 0) / distances.length;
      assert.ok(meanDistance < (kind === "wave" ? 15 : 4), `${width}px mean target error ${meanDistance}`);
      if (kind !== "wave") {
        assert.ok(distances.filter(distance => distance < 8).length >= distances.length * 0.9);
      }
      while (field.formation) field.step(null);
      assert.ok(field.driftRemaining >= 15 && field.driftRemaining <= 25);
      for (let tick = 0; tick < 300; tick += 1) field.step(null);
      const meanSpeed = field.tiles.reduce((total, tile) => total + Body.getSpeed(tile.body), 0) / field.tiles.length;
      assert.ok(meanSpeed > 0.1 && meanSpeed < 0.35);
    }
  });
}

test("traveling waves move their targets; geometric waves remain still", () => {
  const field = new TileField(736, 900);
  field.startFormation("wave", "wave");
  let assignment = [...field.formation.assignments.values()][0];
  const start = field.formationTarget(assignment);
  field.formation.age = field.formation.gather + 2;
  assert.ok(Math.abs(field.formationTarget(assignment).y - start.y) > 10);
  field.startFormation("geometry", "wave");
  assignment = [...field.formation.assignments.values()][0];
  field.formation.age = field.formation.gather + 2;
  assert.deepEqual(field.formationTarget(assignment), assignment);
});

test("cursor breaks nearby formation members without recapturing them", () => {
  const field = new TileField(736, 900);
  field.startFormation("grid");
  const [tile, target] = [...field.formation.assignments][0];
  Body.setPosition(tile.body, target);
  Body.setVelocity(tile.body, { x: -tile.speed, y: 0 });
  const count = field.formation.assignments.size;
  for (let tick = 0; tick < 20; tick += 1) field.step({ x: target.x - 25, y: target.y });
  assert.ok(!field.formation.assignments.has(tile));
  assert.ok(field.formation.assignments.size < count);
  assert.ok(field.formation.assignments.size > 0);
  assert.ok(tile.body.velocity.x > tile.speed);
  for (let tick = 0; tick < 120; tick += 1) field.step(null);
  assert.ok(!field.formation.assignments.has(tile));
});

test("simulation-time scheduling randomly covers every family and geometry", () => {
  const field = new TileField(736, 900);
  const kinds = new Set();
  const shapes = new Set();
  for (let cycle = 0; cycle < 120; cycle += 1) {
    assert.ok(field.driftRemaining >= 15 && field.driftRemaining <= 25);
    field.driftRemaining = 0;
    field.step(null);
    kinds.add(field.formation.kind);
    if (field.formation.kind === "geometry") shapes.add(field.formation.shape);
    field.formation.age = field.formation.gather + field.formation.hold + field.formation.release;
    field.step(null);
    assert.equal(field.formation, null);
  }
  assert.deepEqual([...kinds].sort(), ["clusters", "geometry", "grid", "wave"]);
  assert.deepEqual([...shapes].sort(), ["ring", "square", "wave"]);
  field.startFormation("grid");
  field.resize(343, 740);
  assert.equal(field.formation, null);
  assert.ok(field.driftRemaining >= 15 && field.driftRemaining <= 25);
});