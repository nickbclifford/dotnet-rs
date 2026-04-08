using System;

public class Program {
    private static readonly string[] Payloads = new string[] {
        "{\"id\":11,\"values\":{\"a\":12,\"b\":13,\"c\":14}}",
        "{\"id\":21,\"values\":{\"a\":22,\"b\":23,\"c\":24}}",
        "{\"id\":31,\"values\":{\"a\":32,\"b\":33,\"c\":34}}",
        "{\"id\":41,\"values\":{\"a\":42,\"b\":43,\"c\":44}}",
        "{\"id\":51,\"values\":{\"a\":52,\"b\":53,\"c\":54}}",
    };

    public static int Main() {
        int checksum = 0;

        for (int i = 0; i < 300; i++) {
            string json = Payloads[i % Payloads.Length];
            checksum += ParseIntField(json, "\"id\":");
            checksum += ParseIntField(json, "\"a\":");
            checksum += ParseIntField(json, "\"b\":");
            checksum += ParseIntField(json, "\"c\":");
        }

        return checksum > 0 ? 0 : 1;
    }

    private static int ParseIntField(string json, string key) {
        int keyStart = -1;
        int max = json.Length - key.Length;
        for (int i = 0; i <= max; i++) {
            bool matches = true;
            for (int j = 0; j < key.Length; j++) {
                if (json[i + j] != key[j]) {
                    matches = false;
                    break;
                }
            }
            if (matches) {
                keyStart = i;
                break;
            }
        }

        if (keyStart < 0) {
            return 0;
        }

        int valueStart = keyStart + key.Length;
        int valueEnd = valueStart;
        while (valueEnd < json.Length && json[valueEnd] >= '0' && json[valueEnd] <= '9') {
            valueEnd++;
        }

        int value = 0;
        for (int i = valueStart; i < valueEnd; i++) {
            value = (value * 10) + (json[i] - '0');
        }
        return value;
    }
}
