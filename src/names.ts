import words from "../shared/device-names.json";

const automaticKey = "linchuan-browser-animal-name";
const legacyAutomaticKey = "linchuan-browser-auto-name";
const preferredKey = "linchuan-browser-name";

function storedName(key: string): string {
  try {
    const name = localStorage.getItem(key)?.trim() || "";
    return name &&
      [...name].length <= 64 &&
      !/[\u0000-\u001f\u007f-\u009f]/.test(name)
      ? name
      : "";
  } catch {
    return "";
  }
}

function storeName(key: string, name: string) {
  try {
    localStorage.setItem(key, name);
  } catch {
    // Pairing still works when browser storage is unavailable.
  }
}

export function browserIdentity() {
  let automatic = storedName(automaticKey);
  if (!automatic) {
    automatic = randomNickname();
    storeName(automaticKey, automatic);
  }
  return {
    automatic,
    preferred:
      storedName(preferredKey) || storedName(legacyAutomaticKey) || automatic,
  };
}

export function rememberBrowserName(name: string) {
  storeName(preferredKey, name);
}

export function randomNickname(exclude: string[] = []) {
  const total = words.moods.length * words.animals.length;
  const start = crypto.getRandomValues(new Uint32Array(1))[0] % total;
  for (let offset = 0; offset < total; offset++) {
    const index = (start + offset) % total;
    const name =
      words.moods[Math.floor(index / words.animals.length)] +
      words.animals[index % words.animals.length];
    if (!exclude.includes(name)) return name;
  }
  throw new Error("暂时没有可用昵称");
}

export function deviceLabels(devices: { id: string; name: string }[]) {
  const unique = new Map(devices.map((d) => [d.id, d.name]));
  const groups = new Map<string, string[]>();
  for (const [id, name] of unique)
    groups.set(name, [...(groups.get(name) || []), id]);
  const labels = new Map<string, string>();
  for (const [name, ids] of groups) {
    for (const id of ids) {
      let label = name;
      if (ids.length > 1) {
        let length = 4;
        while (
          length < id.length &&
          ids.some(
            (other) =>
              other !== id &&
              other.slice(-length).toUpperCase() ===
                id.slice(-length).toUpperCase(),
          )
        )
          length += 4;
        label += ` · ${id.slice(-length).toUpperCase()}`;
      }
      labels.set(id, label);
    }
  }
  return labels;
}
