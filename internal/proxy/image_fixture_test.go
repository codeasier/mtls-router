package proxy

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const (
	fixtureDir     = "testdata/9router-image"
	fixtureCommit  = "6fcd27337a7893642c7fe630840d0a641743f28f"
	fixtureVersion = "v0.5.45"
	presetModelCX  = "cx/gpt-5.5-image"
	presetModelAG  = "ag/gemini-3.1-flash-image"
	fixtureB64PNG  = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEElEQVR4nGP4z8AARwzEcQCukw/x0F8jngAAAABJRU5ErkJggg=="
	pngMagicHex    = "89504e470d0a1a0a"
)

type fixtureSource struct {
	Source struct {
		Repository string `json:"repository"`
		Version    string `json:"version"`
		Commit     string `json:"commit"`
		CommitURL  string `json:"commitUrl"`
	} `json:"source"`
}

type modelsListFixture struct {
	Object string `json:"object"`
	Data   []struct {
		ID      string `json:"id"`
		Object  string `json:"object"`
		OwnedBy string `json:"owned_by"`
	} `json:"data"`
}

type generationRequestFixture struct {
	Model  string `json:"model"`
	Prompt string `json:"prompt"`
	N      int    `json:"n"`
	Image  string `json:"image,omitempty"`
}

type generationResponseFixture struct {
	Created int64 `json:"created"`
	Data    []struct {
		B64JSON string `json:"b64_json"`
	} `json:"data"`
}

func loadFixture(t *testing.T, name string) []byte {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(fixtureDir, name))
	if err != nil {
		t.Fatalf("read fixture %s: %v", name, err)
	}
	return data
}

func TestFixtureSourceMetadata(t *testing.T) {
	var src fixtureSource
	if err := json.Unmarshal(loadFixture(t, "source.json"), &src); err != nil {
		t.Fatalf("parse source.json: %v", err)
	}
	if src.Source.Repository != "decolua/9router" {
		t.Errorf("repository = %q", src.Source.Repository)
	}
	if src.Source.Version != fixtureVersion {
		t.Errorf("version = %q, want %q", src.Source.Version, fixtureVersion)
	}
	if src.Source.Commit != fixtureCommit {
		t.Errorf("commit = %q, want %q", src.Source.Commit, fixtureCommit)
	}
	if !strings.Contains(src.Source.CommitURL, src.Source.Commit) {
		t.Errorf("commitUrl %q does not contain commit %q", src.Source.CommitURL, src.Source.Commit)
	}
}

func TestModelsImageFixtureContainsPresetIDs(t *testing.T) {
	var catalog modelsListFixture
	if err := json.Unmarshal(loadFixture(t, "models-image-response.json"), &catalog); err != nil {
		t.Fatalf("parse models-image-response.json: %v", err)
	}
	if catalog.Object != "list" {
		t.Errorf("object = %q, want \"list\"", catalog.Object)
	}
	ids := make(map[string]bool, len(catalog.Data))
	for _, m := range catalog.Data {
		if m.Object != "model" {
			t.Errorf("model %q has object %q, want \"model\"", m.ID, m.Object)
		}
		if !strings.Contains(m.ID, "/") {
			t.Errorf("model ID %q does not contain slash", m.ID)
		}
		ids[m.ID] = true
	}
	if !ids[presetModelCX] {
		t.Errorf("preset model %q missing from catalog", presetModelCX)
	}
	if !ids[presetModelAG] {
		t.Errorf("preset model %q missing from catalog", presetModelAG)
	}
}

func TestGenerationRequestFixtureSchema(t *testing.T) {
	for _, name := range []string{"generation-cx-request.json", "generation-ag-request.json"} {
		t.Run(name, func(t *testing.T) {
			var req generationRequestFixture
			if err := json.Unmarshal(loadFixture(t, name), &req); err != nil {
				t.Fatalf("parse %s: %v", name, err)
			}
			if req.Model == "" {
				t.Error("model is empty")
			}
			if !strings.Contains(req.Model, "/") {
				t.Errorf("model %q does not contain slash", req.Model)
			}
			if req.Prompt == "" {
				t.Error("prompt is empty")
			}
			if req.N != 1 {
				t.Errorf("n = %d, want 1", req.N)
			}
			if req.Image != "" {
				t.Errorf("generation request has image field %q, should be absent", req.Image)
			}
		})
	}
}

func TestEditRequestFixtureSchema(t *testing.T) {
	for _, name := range []string{"edit-cx-request.json", "edit-ag-request.json"} {
		t.Run(name, func(t *testing.T) {
			var req generationRequestFixture
			if err := json.Unmarshal(loadFixture(t, name), &req); err != nil {
				t.Fatalf("parse %s: %v", name, err)
			}
			if req.Model == "" {
				t.Error("model is empty")
			}
			if req.Prompt == "" {
				t.Error("prompt is empty")
			}
			if req.N != 1 {
				t.Errorf("n = %d, want 1", req.N)
			}
			if req.Image == "" {
				t.Error("edit request image field is empty")
			}
			if !strings.HasPrefix(req.Image, "data:image/png;base64,") {
				t.Errorf("image field is not a PNG data URI: %q", req.Image[:min(len(req.Image), 40)])
			}
		})
	}
}

func TestGenerationResponseFixtureSchema(t *testing.T) {
	for _, name := range []string{
		"generation-cx-response.json",
		"generation-ag-response.json",
		"edit-cx-response.json",
		"edit-ag-response.json",
	} {
		t.Run(name, func(t *testing.T) {
			var resp generationResponseFixture
			if err := json.Unmarshal(loadFixture(t, name), &resp); err != nil {
				t.Fatalf("parse %s: %v", name, err)
			}
			if resp.Created <= 0 {
				t.Error("created is not positive")
			}
			if len(resp.Data) != 1 {
				t.Fatalf("data has %d items, want 1", len(resp.Data))
			}
			if resp.Data[0].B64JSON == "" {
				t.Fatal("b64_json is empty")
			}
			decoded, err := base64.StdEncoding.DecodeString(resp.Data[0].B64JSON)
			if err != nil {
				t.Fatalf("b64_json is not valid base64: %v", err)
			}
			if len(decoded) < 8 {
				t.Fatalf("decoded image is only %d bytes", len(decoded))
			}
			if fmt.Sprintf("%x", decoded[:8]) != pngMagicHex {
				t.Errorf("decoded image magic bytes = %x, want %s", decoded[:8], pngMagicHex)
			}
		})
	}
}

func TestFixtureModelIDDriftFailsClosed(t *testing.T) {
	var catalog modelsListFixture
	if err := json.Unmarshal(loadFixture(t, "models-image-response.json"), &catalog); err != nil {
		t.Fatalf("parse models-image-response.json: %v", err)
	}
	ids := make(map[string]bool, len(catalog.Data))
	for _, m := range catalog.Data {
		ids[m.ID] = true
	}
	drifted := []string{
		"cx/gpt-5.6-image",
		"cx/gpt-5.5-image-v2",
		"ag/gemini-3.2-flash-image",
		"ag/gemini-3.1-flash-image-pro",
		"cx/gpt5.5-image",
		"cx\\gpt-5.5-image",
	}
	for _, id := range drifted {
		if ids[id] {
			t.Errorf("drifted model ID %q should not match any fixture entry", id)
		}
	}
}

func TestFixturesContainNoCredentials(t *testing.T) {
	entries, err := os.ReadDir(fixtureDir)
	if err != nil {
		t.Fatalf("read fixture dir: %v", err)
	}
	sensitiveMarkers := []string{
		"api_key", "apiKey", "API_KEY",
		"Authorization", "authorization",
		"Bearer ", "bearer ",
		"sk-", "secret", "token",
		"password",
	}
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		if name == "sample.png" || name == "source.json" {
			continue
		}
		data := loadFixture(t, name)
		text := string(data)
		for _, marker := range sensitiveMarkers {
			if strings.Contains(strings.ToLower(text), strings.ToLower(marker)) {
				t.Errorf("fixture %s contains sensitive marker %q", name, marker)
			}
		}
	}
}

func TestSamplePngIsValidImage(t *testing.T) {
	data := loadFixture(t, "sample.png")
	if len(data) < 8 {
		t.Fatalf("sample.png is only %d bytes", len(data))
	}
	if fmt.Sprintf("%x", data[:8]) != pngMagicHex {
		t.Errorf("sample.png magic bytes = %x, want %s", data[:8], pngMagicHex)
	}
	b64 := base64.StdEncoding.EncodeToString(data)
	if b64 != fixtureB64PNG {
		t.Errorf("sample.png base64 = %q, want %q", b64, fixtureB64PNG)
	}
}
