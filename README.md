# Nutler - Company Network Chat Application

A modern, secure chat application designed for company networks with department-based chat rooms and real-time messaging.

## 🚀 Features

### Core Features
- **Department-based Chat Rooms**: Organized chat rooms for each department (IT, HR, Finance, Marketing, Operations)
- **Real-time Messaging**: Instant message delivery with emoji support
- **User Management**: User registration with department assignment
- **Network Discovery**: Automatic server discovery on local networks
- **Persistent Messages**: SQLite database for message history
- **Modern UI**: Beautiful, responsive interface built with React and Tailwind CSS

### Technical Features
- **Cross-platform Desktop App**: Built with Tauri (Rust + React)
- **Local Network Support**: Works on company intranets without internet
- **Secure Communication**: Direct TCP socket communication
- **Database Persistence**: SQLite for user and message storage
- **Multi-client Support**: Multiple users can connect to the same server

## 🛠️ Technology Stack

- **Frontend**: React 19, TypeScript, Tailwind CSS, Lucide React
- **Backend**: Rust, Tauri 2.0, SQLite, TCP Sockets
- **Database**: SQLite with SQLx for async operations
- **Build Tool**: Vite

## 📋 Prerequisites

- Node.js 18+ 
- Rust (latest stable)
- Git

## 🚀 Quick Start

### 1. Clone and Install

```bash
git clone <repository-url>
cd nutler
npm install
```

### 2. Development Mode

```bash
npm run tauri dev
```

This will start the development server and open the application.

### 3. Build for Production

```bash
npm run tauri build
```

This creates distributable packages for your platform.

## 🏢 Company Network Setup

### Server Setup (IT Department)

1. **Deploy the Application**: Install the built application on a server machine
2. **Start Server Mode**: 
   - Open the application
   - Select "Host Server" mode
   - The server will bind to `0.0.0.0:3625` (accessible from all network interfaces)
3. **Note the IP Address**: The server will display its IP address (e.g., `192.168.1.100:3625`)

### Client Setup (All Users)

1. **Install Application**: Deploy the application to all user machines
2. **User Registration**:
   - Enter name, email, and select department
   - The system will create a user account
3. **Connect to Server**:
   - Select "Join Server" mode
   - Enter the server IP address (e.g., `192.168.1.100:3625`)
   - Or use the "Discover Servers" feature to find servers automatically

## 🏗️ Architecture

### Database Schema

```
departments
├── id (PK)
├── name
└── description

users
├── id (PK)
├── name
├── email
├── department_id (FK)
├── is_online
└── last_seen

chat_rooms
├── id (PK)
├── name
├── description
├── department_id (FK)
├── is_private
└── created_by (FK)

user_rooms (Many-to-Many)
├── id (PK)
├── user_id (FK)
├── room_id (FK)
├── joined_at
└── is_active

messages
├── id (PK)
├── room_id (FK)
├── user_id (FK)
├── message
├── message_type
├── is_emoji
└── created_at
```

### Network Communication

- **Protocol**: Custom TCP-based messaging protocol
- **Port**: 3625 (configurable)
- **Message Format**: JSON with type, user, room, and content information
- **Discovery**: Automatic scanning of common local network ranges

## 🔧 Configuration

### Default Departments

The application comes with pre-configured departments:
- **IT**: Information Technology Department
- **HR**: Human Resources Department  
- **Finance**: Finance and Accounting Department
- **Marketing**: Marketing and Sales Department
- **Operations**: Operations Department
- **General**: General Company Chat

### Default Chat Rooms

Each department gets a general chat room:
- IT General
- HR General
- Finance General
- Marketing General
- Operations General
- Company Wide (for all departments)

## 🚀 Deployment Guide

### For IT Administrators

1. **Build the Application**:
   ```bash
   npm run tauri build
   ```

2. **Deploy to Server**:
   - Copy the built application to a server machine
   - Ensure the server machine is accessible on the company network
   - Configure firewall to allow connections on port 3625

3. **Start the Server**:
   - Run the application on the server
   - Select "Host Server" mode
   - Note the displayed IP address

4. **Deploy to Users**:
   - Distribute the application to all users
   - Provide the server IP address to users
   - Or instruct users to use the "Discover Servers" feature

### Network Requirements

- **Port**: 3625 (TCP)
- **Protocol**: TCP
- **Network Access**: All machines must be on the same local network
- **Firewall**: Ensure port 3625 is open on the server

## 🔒 Security Considerations

- **Local Network Only**: Application is designed for internal company networks
- **No Internet Required**: All communication happens within the local network
- **User Authentication**: Basic user registration with department assignment
- **Message Persistence**: Messages are stored locally in SQLite database

## 🐛 Troubleshooting

### Common Issues

1. **Cannot Connect to Server**:
   - Verify server IP address is correct
   - Check firewall settings on server
   - Ensure both machines are on the same network

2. **Server Not Found**:
   - Use "Discover Servers" feature
   - Manually enter server IP address
   - Check if server is running and accessible

3. **Database Errors**:
   - Application will automatically create database on first run
   - Check file permissions in application data directory

### Debug Information

- Check console output for connection information
- Server displays its IP address when started
- Client shows connection status in the interface

## 📝 Development

### Project Structure

```
nutler/
├── src/                    # React frontend
│   ├── App.tsx           # Main application
│   ├── Chat.tsx          # Chat interface
│   └── ...
├── src-tauri/            # Rust backend
│   ├── src/
│   │   ├── lib.rs        # Main application logic
│   │   ├── sockets.rs    # Network communication
│   │   ├── db_queries.rs # Database operations
│   │   └── migration.rs  # Database schema
│   └── ...
└── ...
```

### Adding New Features

1. **New Database Tables**: Add migrations in `src-tauri/src/migration.rs`
2. **New API Commands**: Add functions in `src-tauri/src/db_queries.rs`
3. **UI Changes**: Modify React components in `src/`
4. **Network Features**: Extend `src-tauri/src/sockets.rs`

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Test thoroughly
5. Submit a pull request

## 📄 License

This project is licensed under the MIT License.

## 🆘 Support

For support and questions:
- Check the troubleshooting section above
- Review the console output for error messages
- Ensure network connectivity between machines

---

**Note**: This application is designed for internal company networks and does not require internet connectivity. All communication happens within the local network infrastructure.
