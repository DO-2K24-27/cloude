import sys
import platform

print("Hello from Python Agent!")
print(f"Python Version: {platform.python_version()}")
print(f"OS Platform: {platform.platform()}")

# Try a small math operation
res = sum(range(1, 101))
print(f"Sum 1 to 100 is: {res}")

sys.exit(0)
