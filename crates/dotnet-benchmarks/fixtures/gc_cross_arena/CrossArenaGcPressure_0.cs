using System;

public class Program {
    private const int SlotCount = 64;
    private const int Iterations = 300;
    private const int ChainLength = 4;
    private const int PayloadSize = 32;

    private static readonly Node?[] SharedRoots = new Node?[SlotCount];

    private sealed class Node {
        public int Marker;
        public byte[] Payload;
        public Node? Next;

        public Node(int marker) {
            Marker = marker;
            Payload = new byte[PayloadSize];
            for (int i = 0; i < Payload.Length; i++) {
                Payload[i] = (byte)((marker + i * 13) & 0xFF);
            }
        }
    }

    private static Node BuildChain(int seed) {
        Node head = new Node(seed);
        Node current = head;

        for (int i = 1; i < ChainLength; i++) {
            Node next = new Node(seed + i);
            current.Next = next;
            current = next;
        }

        return head;
    }

    public static int Main() {
        int checksum = 0;

        for (int i = 0; i < SharedRoots.Length; i++) {
            SharedRoots[i] = null;
        }

        for (int i = 0; i < Iterations; i++) {
            Node chain = BuildChain(i * 17 + 3);
            int slot = i & (SlotCount - 1);
            int clearSlot = (i * 11 + 7) & (SlotCount - 1);

            // Shared static roots make this fixture suitable for multi-arena
            // cross-arena GC runs while still functioning in single-arena mode.
            SharedRoots[slot] = chain;
            if ((i & 7) == 0) {
                SharedRoots[clearSlot] = null;
            }

            Node? cursor = chain;
            int depth = 0;
            while (cursor != null && depth < (ChainLength + 4)) {
                int payloadIndex = (i + depth * 19) & (PayloadSize - 1);
                checksum ^= cursor.Marker;
                checksum += cursor.Payload[payloadIndex];
                checksum &= 0x7FFF_FFFF;
                cursor = cursor.Next;
                depth++;
            }

        }

        int survivors = 0;
        for (int i = 0; i < SharedRoots.Length; i++) {
            if (SharedRoots[i] != null) {
                survivors++;
            }
        }

        return checksum != 0 && survivors > 0 ? 0 : 2;
    }
}
