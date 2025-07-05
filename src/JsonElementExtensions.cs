using System.Text.Json;
using System.Collections.Generic;
using System.Linq;

namespace UPD.Extensions
{
    public static class JsonElementExtensions
    {
        public static string GetPropertyStringOrDefault(this JsonElement? elem, string prop)
        {
            if (elem == null || elem.Value.ValueKind == JsonValueKind.Undefined) return "";
            if (elem.Value.TryGetProperty(prop, out var v))
            {
                if (v.ValueKind == JsonValueKind.String) return v.GetString() ?? "";
                if (v.ValueKind == JsonValueKind.Number) return v.ToString();
                if (v.ValueKind == JsonValueKind.Null) return "";
                return v.ToString();
            }
            return "";
        }
        public static IEnumerable<KeyValuePair<string, string>> GetPropertyDictOrDefault(this JsonElement? elem, string prop)
        {
            if (elem == null || elem.Value.ValueKind == JsonValueKind.Undefined) yield break;
            if (elem.Value.TryGetProperty(prop, out var val) && val.ValueKind == JsonValueKind.Object)
            {
                foreach (var x in val.EnumerateObject())
                {
                    yield return new KeyValuePair<string, string>(x.Name, x.Value.GetString() ?? x.Value.ToString());
                }
            }
            else if (elem.Value.TryGetProperty(prop, out var arr) && arr.ValueKind == JsonValueKind.Array)
            {
                int i = 0;
                foreach (var x in arr.EnumerateArray())
                {
                    yield return new KeyValuePair<string, string>($"[{i++}]", x.ToString());
                }
            }
        }
        public static IEnumerable<JsonElement> GetPropertyArrayOrDefault(this JsonElement? elem, string prop)
        {
            if (elem == null || elem.Value.ValueKind == JsonValueKind.Undefined) yield break;
            if (elem.Value.TryGetProperty(prop, out var arr) && arr.ValueKind == JsonValueKind.Array)
            {
                foreach (var x in arr.EnumerateArray())
                {
                    yield return x;
                }
            }
        }
        public static string GetEnvVarsString(this JsonElement? elem)
        {
            if (elem == null || elem.Value.ValueKind == JsonValueKind.Undefined) return "";
            if (elem.Value.TryGetProperty("env_vars", out var obj) && obj.ValueKind == JsonValueKind.Object)
            {
                return string.Join("\n", obj.EnumerateObject().Select(x => $"{x.Name}={x.Value.GetString() ?? x.Value.ToString()}"));
            }
            return "";
        }
        public static IEnumerable<KeyValuePair<string, string>> GetEnvVarsDict(this JsonElement? elem)
        {
            if (elem == null || elem.Value.ValueKind == JsonValueKind.Undefined) yield break;
            if (elem.Value.TryGetProperty("env_vars", out var obj) && obj.ValueKind == JsonValueKind.Object)
            {
                foreach (var x in obj.EnumerateObject())
                {
                    yield return new KeyValuePair<string, string>(x.Name, x.Value.GetString() ?? x.Value.ToString());
                }
            }
        }
    }
} 