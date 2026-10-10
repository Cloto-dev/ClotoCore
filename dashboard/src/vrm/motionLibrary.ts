export const MOTION_SOURCES = [
  'Meshy',
  'Kimodo',
  'ARDY',
  'Cascadeur',
  'iClone / AccuPose',
  'DeepMotion',
  'Rokoko',
  'Flow Studio',
  'Blender Agent Skill',
  'dcc-mcp',
  'blender-claude-plugin',
  'MotionMCP',
  'SAM 3D Body',
  'Other',
] as const;
export interface MotionEntry {
  id: string;
  name: string;
  source: string;
  conditions: string;
  filename: string;
  file: Blob;
  sha256: string;
  duration: number;
  bones: number;
  fps: number;
  added: string;
  reviews: Record<string, { avatar: string; notes: string }>;
}

const DB_NAME = 'mizmate-motion-library';
const STORE = 'motions';
const LIMIT = 256 * 1024 * 1024;

async function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (!globalThis.indexedDB) {
      reject(new Error('Local storage unavailable'));
      return;
    }
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: 'id' });
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error('Local storage blocked'));
    request.onsuccess = () => resolve(request.result);
  });
}

export async function listMotions(): Promise<MotionEntry[]> {
  const db = await database();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readonly');
    const request = tx.objectStore(STORE).getAll();
    tx.oncomplete = () => {
      db.close();
      resolve((request.result as MotionEntry[]).sort((a, b) => a.added.localeCompare(b.added)));
    };
    tx.onabort = () => {
      db.close();
      reject(tx.error);
    };
  });
}

export async function saveMotion(entry: MotionEntry): Promise<void> {
  const db = await database();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    const store = tx.objectStore(STORE);
    const read = store.getAll();
    let problem: Error | null = null;
    read.onsuccess = () => {
      const others = (read.result as MotionEntry[]).filter((row) => row.id !== entry.id);
      if (others.length >= 64 || others.reduce((sum, row) => sum + row.file.size, entry.file.size) > LIMIT) {
        problem = new Error('Motion library is full');
        tx.abort();
        return;
      }
      store.put(entry);
    };
    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onabort = () => {
      db.close();
      reject(problem ?? tx.error);
    };
  });
}

export async function removeMotion(id: string): Promise<void> {
  const db = await database();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    tx.objectStore(STORE).delete(id);
    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onabort = () => {
      db.close();
      reject(tx.error);
    };
  });
}

/** Merge one avatar's observations against the current row, including writes from other windows. */
export async function saveMotionReview(
  id: string,
  agentId: string,
  avatar: string,
  notes: string,
  fps: number,
): Promise<void> {
  const db = await database();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, 'readwrite');
    const store = tx.objectStore(STORE);
    const request = store.get(id);
    request.onsuccess = () => {
      const row = request.result as MotionEntry | undefined;
      if (!row) {
        tx.abort();
        return;
      }
      store.put({ ...row, fps, reviews: { ...row.reviews, [agentId]: { avatar, notes } } });
    };
    tx.oncomplete = () => {
      db.close();
      resolve();
    };
    tx.onabort = () => {
      db.close();
      reject(tx.error ?? new Error('Motion was removed'));
    };
  });
}

export function motionReport(entries: MotionEntry[]): string {
  return JSON.stringify(
    {
      schema: 'mizmate-motion-comparison-v1',
      motions: entries.map(({ file, ...entry }) => ({ ...entry, bytes: file.size })),
    },
    null,
    2,
  );
}
