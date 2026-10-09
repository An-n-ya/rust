#!/usr/bin/env python3
# -*- coding: utf-8 -*-

import requests

def get_crates_in_category(category_id):
    crates = []
    page = 1
    url = f"https://crates.io/api/v1/crates"

    while True:
        params = {
            "category": category_id,
            "per_page": 100,
            "page": page,
            "sort": "downloads"
        }

        headers = {
            "User-Agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
        }

        response = requests.get(url, params=params, headers=headers, timeout=10)
        print(response.url)
        if response.status_code != 200:
            print(f"Error: {response.status_code}")
            break

        data = response.json()
        page_crates = data.get("crates", [])
        
        if not page_crates:
            break

        crates.extend(crate["name"] for crate in page_crates)
        print(f"Fetched page {page}, total so far: {len(crates)}")

        page += 1

    return crates

if __name__ == "__main__":
    api_binding_crates = get_crates_in_category("external-ffi-bindings")
    print(f"\nTotal crates in 'external-ffi-bindings': {len(api_binding_crates)}")
    print("write to file...")

    with open("external-ffi-bindings_crates.txt", "w") as f:
        for crate in api_binding_crates:
            f.write(crate + "\n")
