# Marie LookApp

Windows system tray application for searching and inserting phrases from the Marie library.

## Features

- **System Tray**: Runs quietly in the background
- **Global Hotkey**: Press `Ctrl+Shift+L` to open search popup
- **Fuzzy Search**: Find phrases by typing partial matches
- **Quick Insert**: Select a phrase to paste it at your cursor position
- **Cloud Sync**: Syncs with your Marie library via email authentication

## Installation

Download the appropriate installer for your system:
- `MarieLookApp-x64-Setup.exe` - For Intel/AMD Windows PCs
- `MarieLookApp-arm64-Setup.exe` - For ARM Windows PCs (Surface Pro X, etc.)

> **Note**: Windows SmartScreen may show a warning for unsigned executables. Click "More info" → "Run anyway" to proceed.

## Usage

1. **Login**: Enter your email to receive an OTP code
2. **Search**: Press `Ctrl+Shift+L` anywhere to open the search popup
3. **Select**: Use arrow keys or mouse to select a phrase
4. **Insert**: Press Enter or click to insert the phrase at your cursor

## Building from Source

### Requirements
- .NET 8.0 SDK
- Windows (for WPF)

### Build Commands
```bash
# Build for x64
dotnet publish MarieLookApp/MarieLookApp.csproj -c Release -r win-x64 -p:PublishSingleFile=true -p:SelfContained=true -o publish/win-x64

# Build for ARM64
dotnet publish MarieLookApp/MarieLookApp.csproj -c Release -r win-arm64 -p:PublishSingleFile=true -p:SelfContained=true -o publish/win-arm64
```

### Creating Installers
Requires [Inno Setup](https://jrsoftware.org/isinfo.php):
```bash
# x64 installer
ISCC.exe /DArch=x64 Installer/setup.iss

# ARM64 installer
ISCC.exe /DArch=arm64 Installer/setup.iss
```

## License

Private - All rights reserved
