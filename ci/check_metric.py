import json
import sys

if len(sys.argv) != 3:
    print("Usage: check_metric.py <prom_output> <metric>")
    sys.exit(2)

value = json.loads(sys.argv[1])
metric = sys.argv[2]
result = value.get('data', {}).get('result', [])
if result:
    val = float(result[0]['value'][1])
    if val > 0:
        print(f"Metric {metric} exceeded threshold: {val}")
        sys.exit(1)
print(f"Metric {metric} OK")
