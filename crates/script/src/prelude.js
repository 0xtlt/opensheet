(function (op, logNative, spec, enumSpec) {
  "use strict";
  // What a script sees of OpenSheet: the classes of the ExcelScript object
  // model, built from the table of the bindings, the ExcelScript enums and
  // the console. Everything here is hidden from the script but the three
  // globals set at the end.

  function failure(kind, message) {
    const error = new Error(message);
    Object.defineProperty(error, "__kind", { value: kind });
    return error;
  }
  function unsupported(what) {
    return failure("Unsupported", what + " is not supported in OpenSheet");
  }

  // Names the language itself looks up on any object: they are not members
  // of the API a script asked for.
  const quiet = new Set(["then", "toJSON", "constructor", "toString", "valueOf", "_s", "_c", "__kind"]);

  // What is not in the table: any other member fails when it is called,
  // with its name.
  function missing(owner) {
    return new Proxy(Object.create(null), {
      get(target, name) {
        if (typeof name !== "string" || quiet.has(name)) return undefined;
        return function () {
          throw unsupported(owner + "." + name);
        };
      },
    });
  }

  const protos = [];
  function revive(value) {
    if (value === null) return undefined;
    if (typeof value !== "object") return value;
    if (Array.isArray(value)) return value.map(revive);
    if (typeof value.$ === "number" && Array.isArray(value.s)) {
      const object = Object.create(protos[value.$]);
      object._s = value.s;
      return object;
    }
    // Plain data that holds objects of the model (a ShowAsRule and its
    // base field).
    const plain = {};
    for (const key in value) {
      plain[key] = revive(value[key]);
    }
    return plain;
  }
  function call(classIndex, memberIndex, self, args) {
    const answer = op(classIndex, memberIndex, JSON.stringify([self._s, args]));
    switch (answer.charCodeAt(0)) {
      case 117: // u
        return undefined;
      case 118: // v
        return JSON.parse(answer.slice(1));
      case 111: // o
        return revive(JSON.parse(answer.slice(1)));
      case 100: // d
        return new Date(Number(answer.slice(1)));
      default: {
        const error = JSON.parse(answer.slice(1));
        throw failure(error.k, error.m);
      }
    }
  }

  JSON.parse(spec).forEach(function (cls, classIndex) {
    const proto = Object.create(missing(cls.name));
    cls.members.forEach(function (name, memberIndex) {
      proto[name] = function (...args) {
        return call(classIndex, memberIndex, this, args);
      };
    });
    Object.defineProperty(proto, "_c", { value: cls.name });
    proto.toJSON = function () {
      return { $: classIndex, s: this._s };
    };
    proto.toString = function () {
      return "[object " + cls.name + "]";
    };
    protos.push(proto);
  });

  // ---- ExcelScript -----------------------------------------------------

  const enums = Object.create(null);
  JSON.parse(enumSpec).forEach(function (e) {
    const members = Object.create(null);
    e.members.forEach(function (m) {
      members[m[0]] = m[1];
    });
    enums[e.name] = new Proxy(members, {
      get(target, name) {
        if (typeof name !== "string" || name in target || quiet.has(name)) return target[name];
        throw unsupported("ExcelScript." + e.name + "." + name);
      },
      set() {
        return false;
      },
    });
  });
  const ExcelScript = new Proxy(enums, {
    get(target, name) {
      if (typeof name !== "string" || name in target || quiet.has(name)) return target[name];
      return new Proxy(Object.create(null), {
        get(inner, member) {
          if (typeof member !== "string" || quiet.has(member)) return undefined;
          throw unsupported("ExcelScript." + name);
        },
      });
    },
    set() {
      return false;
    },
  });

  // ---- console -----------------------------------------------------------

  // Values as the log of Excel's code editor shows them: text as it is at
  // the top, quoted inside arrays and objects.
  function className(value) {
    const proto = Object.getPrototypeOf(value);
    if (proto === null || proto === Object.prototype) return "";
    if (typeof value._c === "string") return value._c;
    const made = proto.constructor;
    return made && typeof made.name === "string" ? made.name : "";
  }
  function show(value, depth, seen) {
    switch (typeof value) {
      case "string":
        return depth === 0 ? value : JSON.stringify(value);
      case "undefined":
        return "undefined";
      case "function":
        return "ƒ " + (value.name || "anonymous") + "()";
      case "bigint":
        return value + "n";
      case "symbol":
        return value.toString();
      case "object":
        break;
      default:
        return String(value);
    }
    if (value === null) return "null";
    if (seen.includes(value)) return "[Circular]";
    if (value instanceof Error) return (value.name || "Error") + ": " + value.message;
    if (value instanceof Date) return isNaN(value) ? "Invalid Date" : value.toString();
    if (value instanceof RegExp) return value.toString();
    if (typeof value._c === "string" && Array.isArray(value._s)) return value._c + " {}";
    const inner = seen.concat([value]);
    if (Array.isArray(value)) {
      if (depth > 3) return "Array(" + value.length + ")";
      const items = value.slice(0, 100).map(function (item) {
        return show(item, depth + 1, inner);
      });
      if (value.length > 100) items.push("…");
      return "(" + value.length + ") [" + items.join(", ") + "]";
    }
    if (value instanceof Map || value instanceof Set) {
      const kind = value instanceof Map ? "Map" : "Set";
      if (depth > 3) return kind + "(" + value.size + ")";
      const items = Array.from(value)
        .slice(0, 100)
        .map(function (item) {
          return kind === "Map"
            ? show(item[0], depth + 1, inner) + " => " + show(item[1], depth + 1, inner)
            : show(item, depth + 1, inner);
        });
      return kind + "(" + value.size + ") {" + items.join(", ") + "}";
    }
    const name = className(value);
    if (depth > 3) return (name ? name + " " : "") + "{…}";
    const keys = Object.keys(value);
    const fields = keys.slice(0, 100).map(function (key) {
      return key + ": " + show(value[key], depth + 1, inner);
    });
    if (keys.length > 100) fields.push("…");
    return (name ? name + " " : "") + "{" + fields.join(", ") + "}";
  }
  function logger(level) {
    return function (...args) {
      const text = args
        .map(function (arg) {
          return show(arg, 0, []);
        })
        .join(" ");
      logNative(level, text, String(new Error().stack));
    };
  }
  const console = {
    log: logger(0),
    info: logger(0),
    debug: logger(0),
    warn: logger(1),
    error: logger(2),
  };

  // ---- what scripts cannot have ----------------------------------------

  function refused(name) {
    return function () {
      throw unsupported(name);
    };
  }

  const names = { ExcelScript: ExcelScript, console: console, fetch: refused("fetch") };
  names.OfficeScript = new Proxy(Object.create(null), {
    get(target, name) {
      if (typeof name !== "string" || quiet.has(name)) return undefined;
      return refused("OfficeScript." + name);
    },
  });
  for (const name of ["setTimeout", "setInterval", "XMLHttpRequest", "WebSocket", "require"]) {
    names[name] = refused(name);
  }
  for (const name in names) {
    Object.defineProperty(globalThis, name, { value: names[name], writable: false, configurable: false });
  }

  // ---- running -----------------------------------------------------------

  return {
    // Call `main`; the state says how it ended once every job has run.
    run(main) {
      const state = { done: false, failed: false, error: undefined, json: undefined };
      const finish = function (value) {
        state.json = value === undefined ? undefined : JSON.stringify(value);
        state.done = true;
      };
      const fail = function (error) {
        state.error = error;
        state.failed = true;
        state.done = true;
      };
      const result = main(revive({ $: 0, s: [] }));
      if (result !== null && typeof result === "object" && typeof result.then === "function") {
        result.then(function (value) {
          try {
            finish(value);
          } catch (error) {
            fail(error);
          }
        }, fail);
      } else {
        finish(result);
      }
      return state;
    },
    // What was thrown, as JSON text.
    describe(thrown) {
      try {
        if (thrown instanceof Error) {
          return JSON.stringify({
            kind: thrown.__kind || "",
            name: String(thrown.name),
            message: String(thrown.message),
            stack: String(thrown.stack || ""),
          });
        }
        return JSON.stringify({ kind: "", name: "", message: show(thrown, 0, []), stack: "" });
      } catch (error) {
        return '{"kind":"","name":"","message":"The script threw something that cannot be shown","stack":""}';
      }
    },
  };
})
