package update

import "runtime"

// StudioLine is Studio's identity to the shared update path, beside
// StudioCatalog because they answer one question together: what this product
// line publishes, and what installing one of them means. It is the line rather
// than a shell, so both read the same one. Every system path here is Studio's
// own, or dpkg would see one package overwriting another's helper.
func StudioLine() Line {
	return Line{
		Members:   []ReleaseMember{{Archive: studioBinName(), Installed: studioBinName(), Mode: 0o700}},
		Launchers: []string{studioBinName()},
		Deb: DebLine{
			Package:      "reasonix-studio",
			HelperPath:   "/usr/lib/reasonix-studio/reasonix-studio-update-helper",
			PolkitAction: "com.esengine.reasonix-studio.update",
		},
		// The identifier is what the bundle swap checks a downloaded bundle
		// against before it replaces anything; it is the Tauri shell's own.
		Mac: MacLine{BundleID: "com.esengine.reasonix-studio", SelfUpdate: true},
		// A per-user installer writes the profile of the account that runs it;
		// starting it elevated would install into the administrator's profile
		// instead of the user's.
		Windows: WindowsLine{Installer: WindowsInstallerPerUser, InstallerArgs: "--updated"},
	}
}

// The single binary name the line's packages install.
func studioBinName() string {
	if runtime.GOOS == "windows" {
		return "reasonix-studio.exe"
	}
	return "reasonix-studio"
}
