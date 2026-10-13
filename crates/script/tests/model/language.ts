// What real scripts are written with: interfaces, generics, enums,
// classes, closures, helpers beside main, async and await.
interface Item {
  name: string;
  qty: number;
  price?: number;
}

enum Level {
  Low,
  Mid = 5,
  High,
}

const enum Mark {
  Ok = "ok",
  Bad = "bad",
}

type Pair<T> = [T, T];

namespace Money {
  export const rate = 1.2;
  export function gross(net: number): number {
    return Math.round(net * rate * 100) / 100;
  }
}

abstract class Shape {
  abstract area(): number;
  describe(): string {
    return `${this.constructor.name} of ${this.area()}`;
  }
}

class Box<T extends Item> extends Shape {
  static made = 0;
  private total = 0;
  readonly #secret = 7;
  constructor(private readonly items: T[], public label: string = "box") {
    super();
    Box.made++;
  }
  add(item: T): this {
    this.items.push(item);
    this.total += item.qty * (item.price ?? 1);
    return this;
  }
  get sum(): number {
    return this.total;
  }
  area(): number {
    return this.items.length * this.#secret;
  }
}

function first<T>(list: T[], fallback: T): T {
  return list[0] ?? fallback;
}

function swap<T>([a, b]: Pair<T>): Pair<T> {
  return [b, a];
}

const double = (n: number): number => n * 2;

async function later<T>(value: T): Promise<T> {
  return await Promise.resolve(value);
}

function* count(to: number): Generator<number> {
  for (let i = 1; i <= to; i++) {
    yield i;
  }
}

async function main(workbook: ExcelScript.Workbook): Promise<{ total: number; names: string[] }> {
  const sheet: ExcelScript.Worksheet = workbook.getActiveWorksheet();
  const values = sheet.getRange("A2:C4").getValues() as (string | number)[][];
  const items: Item[] = values.map(([name, qty, price]) => ({ name: <string>name, qty: qty as number, price: Number(price) }));
  const box = new Box<Item>([]);
  for (const item of items) {
    box.add(item);
  }
  eq(box.sum, 10 * 1.5 + 2 * 12.25 + 5 * 3);
  eq([box.label, Box.made, box.describe()], ["box", 1, "Box of 21"]);
  eq([Level.Low, Level.Mid, Level.High, Level[5]], [0, 5, 6, "Mid"]);
  const mark: Mark = box.sum > 10 ? Mark.Ok : Mark.Bad;
  eq(mark, "ok");
  eq(Money.gross(10), 12);
  eq(first<number>([], 3), 3);
  eq(swap(["a", "b"]), ["b", "a"]);
  eq(await later(double(21)), 42);
  eq(await Promise.all([later(1), later("two")]), [1, "two"]);
  eq([...count(3)], [1, 2, 3]);
  const missing: Item | undefined = items.find(item => item.name === "Nope");
  eq(missing?.price?.toFixed(2) ?? "none", "none");
  const byName = new Map<string, Item>(items.map(item => [item.name, item] as [string, Item]));
  eq(byName.get("Ink")!.qty, 2);
  const { name, ...rest } = items[0];
  eq([name, rest], ["Pen", { qty: 10, price: 1.5 }]);
  let seen = "";
  try {
    null!.toString();
  } catch (error) {
    seen = error instanceof TypeError ? "type" : "other";
  } finally {
    seen += "!";
  }
  eq(seen, "type!");
  const counter = (() => {
    let n = 0;
    return () => ++n;
  })();
  counter();
  eq(counter(), 2);
  label: for (const i of [1, 2, 3]) {
    for (const j of [1, 2, 3]) {
      if (i * j === 4) {
        break label;
      }
    }
  }
  const stamp = new Date(Date.UTC(2024, 11, 31));
  eq(stamp.toISOString().slice(0, 10), "2024-12-31");
  eq(JSON.parse(JSON.stringify({ a: [1, { b: null }] })), { a: [1, { b: null }] });
  eq("Total: " + (0.1 + 0.2).toFixed(2), "Total: 0.30");
  eq(/^(\w+)@(\w+)\.org$/.exec("ann@example.org")?.[2], "example");
  sheet.getRange("E1").setValue(`${items.length} items, ${Money.gross(box.sum)} gross`);
  await later(null);
  return { total: box.sum, names: items.map(item => item.name).sort() };
}
