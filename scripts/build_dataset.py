#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# build dataset 
import json
import tomllib
import tarfile
import os
import time
import threading
import queue

crates_dir = 'crates'

with open('dataset/external-ffi-bindings_crates.txt', 'r') as f:
    root_deps = [line.strip() for line in f.readlines()]


thread_num = 64
dep_map = {}
dep_map_lock = threading.Lock()
task_queue = queue.Queue(maxsize=thread_num)


def get_dependency(toml_string):
    data = tomllib.loads(toml_string)
    deps = []
    if 'dependencies' in data:
        for dep, val in data['dependencies'].items():
            if isinstance(val, dict) and 'package' in val:
                deps.append(val['package'])
            else:
                deps.append(dep)
    return deps


def get_crate_deps(create_path):
    with tarfile.open(create_path, 'r:gz') as tar_ref:
        for member in tar_ref.getmembers():
            if member.name.endswith('Cargo.toml'):
                file_content = tar_ref.extractfile(
                    member).read().decode('utf-8')
                return get_dependency(file_content)
    return []


def next_dep_crate(dep_map, visited):
    next_crates = []
    for name, deps in dep_map.items():
        if name in visited:
            continue
        for dep in deps:
            if dep in visited:
                next_crates.append(name)
                break
    return next_crates


def consumer():
    while True:
        crate_path = task_queue.get()
        if crate_path is None:
            task_queue.task_done()
            break

        try:
            filename = os.path.basename(crate_path)
            crate_name = filename.rsplit('-', 1)[0]
            deps = get_crate_deps(crate_path)
            with dep_map_lock:
                dep_map[crate_name] = deps
        except Exception as e:
            print(f'error in processing {crate_path}, error: {e}')
        finally:
            task_queue.task_done()


def build_deps_map():
    """Build a map of crate dependencies by extracting Cargo.toml files from .crate archives."""
    start = time.time()
    consumers = [threading.Thread(target=consumer) for _ in range(thread_num)]
    for c in consumers:
        c.daemon = True
        c.start()

    for i, filename in enumerate(os.listdir(crates_dir)):
        if not filename.endswith(".crate"):
            continue
        task_queue.put(os.path.join(crates_dir, filename))
        print(f"Processing {i+1} crates", end='\r')

    for _ in range(len(consumers)):
        task_queue.put(None)
    task_queue.join()

    end = time.time()
    print(f"Time taken to extract dependencies: {end - start:.2f} seconds")

    print("write to file...")
    with open("dataset/crate_deps.json", "w", encoding="utf-8") as f:
        json.dump(dep_map, f, indent=4, ensure_ascii=False)


def main():
    with open("dataset/crate_deps.json", "r", encoding="utf-8") as f:
        dep_map = json.load(f)

    visited = set(root_deps)
    dataset = [i for i in root_deps]
    for i in range(1):
        next_crates = next_dep_crate(dep_map, visited)
        print(
            f"Round {i+1}: Found {len(next_crates)} new crates, total visited: {len(visited) + len(next_crates)}")

        visited.update(next_crates)
        dataset.extend(next_crates)

    print(f"Total crates collected: {len(dataset)}")
    with open("dataset/bt_dataset_r1.txt", "w", encoding="utf-8") as f:
        for crate in dataset:
            f.write(crate + "\n")


if __name__ == "__main__":
    # build_deps_map()
    main()
