using System;

public class Program {
    private sealed class Node {
        public int Value;
        public Node? Next;
    }

    public static int Main() {
        int checksum = 0;

        for (int i = 0; i < 5_000; i++) {
            var head = new Node { Value = i };
            var cur = head;
            for (int j = 0; j < 8; j++) {
                var next = new Node { Value = i + j };
                cur.Next = next;
                cur = next;
            }

            var walk = head;
            while (walk != null) {
                checksum += walk.Value & 0x7;
                walk = walk.Next;
            }
        }

        return checksum > 0 ? 0 : 1;
    }
}
