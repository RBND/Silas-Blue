# Silas Blue Headless Server & Web Management Dockerfile
FROM python:3.14-slim

# Prevent Python from buffering stdout/stderr
ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1

WORKDIR /app

# Install system dependencies (curl and git)
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install python dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application source
COPY . .

# Expose Web Management Port
EXPOSE 8080

# Run in headless daemon mode binding web server to all interfaces
CMD ["python", "SilasBlue.py", "--headless", "--web", "--web-host", "0.0.0.0", "--web-port", "8080"]
