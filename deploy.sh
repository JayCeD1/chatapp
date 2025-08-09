#!/bin/bash

# Nutler - Company Network Chat Application Deployment Script
# This script builds and packages the application for distribution

set -e

echo "🚀 Nutler - Company Network Chat Application"
echo "=============================================="

# Check if we're in the right directory
if [ ! -f "package.json" ] || [ ! -f "src-tauri/Cargo.toml" ]; then
    echo "❌ Error: Please run this script from the project root directory"
    exit 1
fi

# Check prerequisites
echo "📋 Checking prerequisites..."

if ! command -v node &> /dev/null; then
    echo "❌ Error: Node.js is not installed"
    exit 1
fi

if ! command -v cargo &> /dev/null; then
    echo "❌ Error: Rust/Cargo is not installed"
    exit 1
fi

echo "✅ Prerequisites check passed"

# Install dependencies
echo "📦 Installing dependencies..."
npm install

# Build the application
echo "🔨 Building application..."
npm run tauri build

echo ""
echo "✅ Build completed successfully!"
echo ""
echo "📁 Distribution files are located in:"
echo "   - src-tauri/target/release/bundle/"
echo ""
echo "🚀 Deployment Instructions:"
echo "   1. Copy the built application to your server machine"
echo "   2. Run the application and select 'Host Server' mode"
echo "   3. Note the displayed IP address (e.g., 192.168.1.100:3625)"
echo "   4. Distribute the application to all users"
echo "   5. Users should select 'Join Server' and enter the server IP"
echo ""
echo "🔧 Network Configuration:"
echo "   - Ensure port 3625 is open on the server firewall"
echo "   - All machines must be on the same local network"
echo "   - Server IP will be displayed when the server starts"
echo ""
echo "📚 For more information, see README.md"
