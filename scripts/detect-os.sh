#!/bin/bash
# Detect OS and set variables

if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "mac"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "linux"
else
    echo "unknown"
fi