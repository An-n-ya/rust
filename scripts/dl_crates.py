#!/usr/bin/env python3
# download crates

import threading
import queue
import os
import json
import requests
import hashlib

index_dir = 'crates.io-index'  # path to the local copy of crates.io index
download_dir = 'crates'  # directory to save downloaded crate files
# URL pattern for downloading crate files
url_pattern = 'https://crates.io/api/v1/crates/{name}/{vers}/download'

counter = 0
counter_lock = threading.Lock()
task_queue = queue.Queue(maxsize=32)

session = requests.Session()
adapter = requests.adapters.HTTPAdapter(
    pool_connections=32,
    pool_maxsize=32,
    max_retries=1
)
session.mount('https://', adapter)


def producer():
    def yield_files_recursive(dir):
        for dirpath, _, files in os.walk(dir):
            for file in files:
                yield os.path.join(dirpath, file)

    dirs = []
    for d in os.listdir(index_dir):
        if d in ['.github', '.git']:
            continue
        path = os.path.join(index_dir, d)
        if os.path.isdir(path):
            dirs.append(path)

    for dir in dirs:
        for file in yield_files_recursive(dir):
            # print(file)
            task_queue.put(file)


def checksum(file_path, buffer_size=65536):
    sha256 = hashlib.sha256()
    with open(file_path, "rb") as f:
        while chunk := f.read(buffer_size):
            sha256.update(chunk)
    return sha256.hexdigest()


def download(json_file):
    data = None
    with open(json_file, 'r') as f:
        for line in f:
            line = line.strip()
            if line:
                data = json.loads(line)

    if data is None:
        raise Exception('no data found in json file')

    name, vers, cksum = data['name'], data['vers'], data['cksum']

    file_path = os.path.join(download_dir, f'{name}-{vers}.crate')
    url = url_pattern.format(name=name, vers=vers)

    if os.path.exists(file_path):
        if checksum(file_path) == cksum:
            return
        os.remove(file_path)

    response = session.get(url, stream=True, timeout=30)
    response.raise_for_status()

    temp_path = file_path + '.tmp'
    with open(temp_path, 'wb') as f:
        for chunk in response.iter_content(chunk_size=8192):
            f.write(chunk)

    if checksum(temp_path) != cksum:
        os.remove(temp_path)
        raise Exception('checksum not match')
    os.rename(temp_path, file_path)


def consumer():
    while True:
        item = task_queue.get()
        if item is None:
            task_queue.task_done()
            break

        with counter_lock:
            global counter
            index = counter
            counter += 1
        print(f'downloading {index} ...', flush=True)
        try:
            download(item)
        except Exception as e:
            print(f'error in processing {item}, error: {e}')
        finally:
            task_queue.task_done()


def main():
    if not os.path.exists(download_dir):
        os.mkdir(download_dir)

    consumers = [threading.Thread(target=consumer) for _ in range(16)]
    for c in consumers:
        c.daemon = True
        c.start()

    producer()

    for _ in range(len(consumers)):
        task_queue.put(None)
    task_queue.join()
    print(counter)
    print('done')


if __name__ == "__main__":
    main()
