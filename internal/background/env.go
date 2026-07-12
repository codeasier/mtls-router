package background

import "strings"

func ChildEnv(env []string) []string {
	child := make([]string, 0, len(env))
	for _, entry := range env {
		name, _, _ := strings.Cut(entry, "=")
		if strings.EqualFold(name, "MTLS_BACKEND") {
			continue
		}
		child = append(child, entry)
	}
	return child
}

// DesktopChildEnv removes every router configuration variable. Desktop
// launches use explicit flags and embedded policy instead of ambient state.
func DesktopChildEnv(env []string) []string {
	child := make([]string, 0, len(env))
	for _, entry := range env {
		name, _, _ := strings.Cut(entry, "=")
		if strings.HasPrefix(strings.ToUpper(name), "MTLS_") {
			continue
		}
		child = append(child, entry)
	}
	return child
}
