package modelconfig

// DeepMerge returns a fresh object. Objects merge recursively; arrays and
// scalars from overlay replace the base leaf. Inputs are not mutated.
func DeepMerge(base, overlay map[string]any) map[string]any {
	result := cloneMap(base)
	for key, value := range overlay {
		if right, ok := value.(map[string]any); ok {
			if left, ok := result[key].(map[string]any); ok {
				result[key] = DeepMerge(left, right)
				continue
			}
		}
		result[key] = cloneValue(value)
	}
	return result
}

func cloneMap(v map[string]any) map[string]any {
	r := make(map[string]any, len(v))
	for k, x := range v {
		r[k] = cloneValue(x)
	}
	return r
}
func cloneValue(v any) any {
	switch x := v.(type) {
	case map[string]any:
		return cloneMap(x)
	case []any:
		r := make([]any, len(x))
		for i, e := range x {
			r[i] = cloneValue(e)
		}
		return r
	default:
		return x
	}
}
