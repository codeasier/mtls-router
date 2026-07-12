//go:build windows

package agent

import (
	"os"
	"path/filepath"
	"testing"
	"unsafe"

	"golang.org/x/sys/windows"
)

func TestPrivateBackupWindowsACLAllowsOnlyCurrentUser(t *testing.T) {
	dir := t.TempDir()
	source := filepath.Join(dir, "settings.json")
	content := []byte(`{"old_key":"sensitive"}`)
	if err := os.WriteFile(source, content, 0o666); err != nil {
		t.Fatal(err)
	}
	backup, err := createPrivateBackup(source, content, 0o666, "bak")
	if err != nil {
		t.Fatal(err)
	}
	assertCurrentUserOnlyACL(t, backup)

	rollback, err := createPrivateBackup(source, []byte(`{"new_key":"sensitive"}`), 0o666, "rollback-test")
	if err != nil {
		t.Fatal(err)
	}
	assertCurrentUserOnlyACL(t, rollback)
}

func TestAtomicReplacePreservesExistingWindowsDACL(t *testing.T) {
	dir := t.TempDir()
	target := filepath.Join(dir, "settings.json")
	if err := os.WriteFile(target, []byte(`{"old":true}`), 0o600); err != nil {
		t.Fatal(err)
	}
	everyone, err := windows.StringToSid("S-1-1-0")
	if err != nil {
		t.Fatal(err)
	}
	token, err := windows.OpenCurrentProcessToken()
	if err != nil {
		t.Fatal(err)
	}
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil {
		t.Fatal(err)
	}
	acl, err := windows.ACLFromEntries([]windows.EXPLICIT_ACCESS{
		{
			AccessPermissions: windows.GENERIC_ALL,
			AccessMode:        windows.GRANT_ACCESS,
			Trustee: windows.TRUSTEE{
				TrusteeForm: windows.TRUSTEE_IS_SID, TrusteeType: windows.TRUSTEE_IS_USER,
				TrusteeValue: windows.TrusteeValueFromSID(user.User.Sid),
			},
		},
		{
			AccessPermissions: windows.GENERIC_READ,
			AccessMode:        windows.GRANT_ACCESS,
			Trustee: windows.TRUSTEE{
				TrusteeForm: windows.TRUSTEE_IS_SID, TrusteeType: windows.TRUSTEE_IS_WELL_KNOWN_GROUP,
				TrusteeValue: windows.TrusteeValueFromSID(everyone),
			},
		},
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if err := windows.SetNamedSecurityInfo(target, windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION,
		nil, nil, acl, nil); err != nil {
		t.Fatal(err)
	}
	if err := writeAtomic(target, []byte(`{"new":true}`), 0o600, false); err != nil {
		t.Fatal(err)
	}
	descriptor, err := windows.GetNamedSecurityInfo(target, windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION)
	if err != nil {
		t.Fatal(err)
	}
	got, _, err := descriptor.DACL()
	if err != nil {
		t.Fatal(err)
	}
	if got == nil || got.AceCount != 2 {
		t.Fatalf("replaced target ACE count = %v, want 2", got)
	}
}

func assertCurrentUserOnlyACL(t *testing.T, path string) {
	t.Helper()
	securityDescriptor, err := windows.GetNamedSecurityInfo(path, windows.SE_FILE_OBJECT,
		windows.DACL_SECURITY_INFORMATION|windows.PROTECTED_DACL_SECURITY_INFORMATION)
	if err != nil {
		t.Fatal(err)
	}
	dacl, _, err := securityDescriptor.DACL()
	if err != nil {
		t.Fatal(err)
	}
	if dacl == nil {
		t.Fatal("private file has no DACL")
	}
	if count := dacl.AceCount; count != 1 {
		t.Fatalf("private file ACE count = %d, want 1", count)
	}
	var allowed *windows.ACCESS_ALLOWED_ACE
	if err := windows.GetAce(dacl, 0, &allowed); err != nil {
		t.Fatal(err)
	}
	allowedSID := (*windows.SID)(unsafe.Pointer(&allowed.SidStart))
	token, err := windows.OpenCurrentProcessToken()
	if err != nil {
		t.Fatal(err)
	}
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil {
		t.Fatal(err)
	}
	if !allowedSID.Equals(user.User.Sid) {
		t.Fatalf("private file ACL belongs to %s, want current user %s", allowedSID, user.User.Sid)
	}
}
