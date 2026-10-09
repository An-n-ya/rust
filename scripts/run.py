#!/usr/bin/env python3
# -*- coding: utf-8 -*-
# This script is used to build and test crates in the dataset.

import os
import subprocess
import tarfile
from sqlalchemy import create_engine, Column, Integer, String, Boolean, DateTime
from sqlalchemy.ext.declarative import declarative_base
from sqlalchemy.orm import sessionmaker
from datetime import datetime
import threading
import queue

Base = declarative_base()


class ResultDB(Base):
    __tablename__ = 'results'

    id = Column(Integer, primary_key=True, autoincrement=True)
    crate_name = Column(String, nullable=False)
    build_status = Column(Boolean, default=False)
    test_status = Column(Boolean)
    found_status = Column(Boolean)
    has_ffi = Column(Boolean)
    timestamp = Column(DateTime, default=datetime.utcnow)


engine = create_engine('sqlite:///results.db')
Session = sessionmaker(bind=engine)
session_lock = threading.Lock()


class Result:
    def __init__(self, crate_name):
        self.crate_name = crate_name
        self.build = False
        self.test = False
        self.found = False
        self.has_ffi = False

    def save_to_db(self):
        with session_lock:
            session = Session()
            result_entry = ResultDB(
                crate_name=self.crate_name,
                build_status=self.build,
                test_status=self.test,
                found_status=self.found,
                has_ffi=self.has_ffi
            )
            session.add(result_entry)
            session.commit()
            session.close()

    def exists_in_db(self):
        with session_lock:
            session = Session()
            existing_record = session.query(ResultDB).filter_by(
                crate_name=self.crate_name).first()
            session.close()
        return existing_record is not None


crates_dir = "crates"
build_dir = "build"


def extract_gzip_file(gzip_path, extract_to):
    os.makedirs(extract_to, exist_ok=True)
    with tarfile.open(gzip_path, 'r:gz') as tar_ref:
        tar_ref.extractall(extract_to, filter='data')


def build_crate(crate_dir, build_log_path):
    result = subprocess.run(
        ["cargo", "+stage1", "test", "--no-run"],
        cwd=crate_dir,
        capture_output=True,
        text=True,
        timeout=1200 # 20 minutes timeout for building
    )
    with open(build_log_path, 'w') as log_file:
        log_file.write(result.stdout)
        log_file.write(result.stderr)
    return result.returncode == 0


def test_crate(crate_dir, run_log_path):
    result = subprocess.run(
        ["cargo", "+stage1", "test"],
        cwd=crate_dir,
        capture_output=True,
        text=True,
        timeout=1200 # 20 minutes timeout for testing
    )
    with open(run_log_path, 'w') as log_file:
        log_file.write(result.stdout)
        log_file.write(result.stderr)
    return result.returncode == 0


def prepend_line(file_path, line_to_add):
    with open(file_path, 'r', encoding='utf-8') as f:
        original_content = f.read()

    with open(file_path, 'w', encoding='utf-8') as f:
        f.write(line_to_add.rstrip('\n') + '\n' + original_content)


def add_feature(crate_dir):
    lib_rs = os.path.join(crate_dir, "src/lib.rs")
    main_rs = os.path.join(crate_dir, "src/main.rs")

    if os.path.exists(lib_rs):
        prepend_line(lib_rs, "#![feature(ffi_sanitizer)]")

    if os.path.exists(main_rs):
        prepend_line(main_rs, "#![feature(ffi_sanitizer)]")


def cleanup_crate(crate_dir):
    subprocess.run(["cargo", "clean"], cwd=crate_dir)


def process(crate_path):
    crate_name = os.path.basename(crate_path)

    result = Result(crate_name)
    if result.exists_in_db():
        print(f"Skipping {crate_name}, already processed.")
        return

    crate_dir = os.path.join("build", crate_name.replace(".crate", ""))
    extract_gzip_file(crate_path, build_dir)

    if os.path.exists(os.path.join(crate_dir, "rust-toolchain")) or os.path.exists(os.path.join(crate_dir, "rust-toolchain.toml")):
        print(f"Skipping {crate_name}, custom toolchain detected.")
        result.save_to_db()
        return

    add_feature(crate_dir)

    print(f"Building for {crate_path}")
    build_log_path = os.path.join(crate_dir, "build.log")
    result.build = build_crate(crate_dir, build_log_path)

    if result.build:
        with open(build_log_path, 'r') as log_file:
            content = log_file.read()
            if "FFISanitizer" in content:
                result.has_ffi = True
        if result.has_ffi:
            print(f"Testing for {crate_name}")
            run_log_path = os.path.join(crate_dir, "run.log")
            result.test = test_crate(crate_dir, run_log_path)

            with open(run_log_path, 'r') as log_file:
                content = log_file.read()
                if "FFI Probe" in content:
                    result.found = True

    if not result.found:
        cleanup_crate(crate_dir)
    result.save_to_db()


task_queue = queue.Queue(maxsize=32)


def consumer():
    while True:
        item = task_queue.get()
        if item is None:
            task_queue.task_done()
            break
        try:
            process(item)
        except KeyboardInterrupt:
            print("Consumer interrupted, exiting.")
            break
        except Exception as e:
            print(f'error in processing {item}, error: {e}')
            result = Result(os.path.basename(item))
            result.save_to_db()
        finally:
            task_queue.task_done()


def main():
    Base.metadata.create_all(engine)
    os.makedirs(build_dir, exist_ok=True)

    consumers = [threading.Thread(target=consumer) for _ in range(8)]
    for c in consumers:
        c.daemon = True
        c.start()

    with open("dataset/bt_dataset_r1.txt", "r", encoding="utf-8") as f:
        crate_names = [line.strip() for line in f.readlines()]
    crate_name_set = set(crate_names)

    path_map = {}
    for filename in os.listdir(crates_dir):
        if not filename.endswith(".crate"):
            continue
        crate_name = filename.rsplit('-', 1)[0]
        if crate_name not in crate_name_set:
            continue
        path_map[crate_name] = os.path.join(crates_dir, filename)

    print(f"Total crates to process: {len(path_map)}")

    for crate_name in crate_names:
        crate_path = path_map.get(crate_name)
        if crate_path:
            task_queue.put(crate_path)

    for _ in range(len(consumers)):
        task_queue.put(None)
    task_queue.join()
    print("All tasks completed.")


if __name__ == "__main__":
    main()
