using System.Runtime.CompilerServices;
using DotnetRs;

namespace DotnetRs
{
    public static class Internal
    {
        [MethodImpl(MethodImplOptions.InternalCall)]
        public static extern ref T GetArrayData<T>(T[] array);
    }
}

// NOTE: These types must be in the System namespace for the C# compiler to allow
// them to be 'ref structs' that interoperate with BCL-expected Span signatures.
// They are still marked with [Stub] so the runtime treats them as the canonical BCL spans.
namespace System
{
    [Stub(InPlaceOf = "System.Span`1")]
    public readonly ref struct Span<T>
    {
        internal readonly ref T _reference;
        internal readonly int _length;

        public Span(T[] array)
        {
            if (array == null)
            {
                _reference = ref Unsafe.NullRef<T>();
                _length = 0;
                return;
            }

            _length = array.Length;
            _reference = ref Internal.GetArrayData(array);
        }

        public Span(T[] array, int start, int length)
        {
            if (array == null)
            {
                if (start != 0 || length != 0)
                    throw new NullReferenceException();
                _reference = ref Unsafe.NullRef<T>();
                _length = 0;
                return;
            }
            
            if ((uint)start > (uint)array.Length || (uint)length > (uint)(array.Length - start))
                throw new ArgumentOutOfRangeException();

            _length = length;
            _reference = ref Unsafe.Add(ref Internal.GetArrayData(array), start);
        }

        public Span(ref T ptr, int length)
        {
            _reference = ref ptr;
            _length = length;
        }

        unsafe public Span(void* ptr, int length)
        {
            _reference = ref Unsafe.AsRef<T>(ptr);
            _length = length;
        }

        public Span(ref T ptr)
        {
            _reference = ref ptr;
            _length = 1;
        }

        public int Length => _length;

        public bool IsEmpty => _length == 0;

        public static Span<T> Empty => default;

        public static implicit operator Span<T>(T[] array) => new Span<T>(array);

        public static implicit operator ReadOnlySpan<T>(Span<T> span) => new ReadOnlySpan<T>(ref span._reference, span._length);

        public void CopyTo(Span<T> destination)
        {
             if ((uint)_length > (uint)destination.Length)
                 throw new ArgumentException();
             
             for (int i = 0; i < _length; i++)
             {
                 destination[i] = this[i];
             }
        }

        public Span<T> Slice(int start)
        {
             if ((uint)start > (uint)_length)
                 throw new ArgumentOutOfRangeException();
             return new Span<T>(ref Unsafe.Add(ref _reference, start), _length - start);
        }

        public Span<T> Slice(int start, int length)
        {
             if ((uint)start > (uint)_length || (uint)length > (uint)(_length - start))
                 throw new ArgumentOutOfRangeException();
             return new Span<T>(ref Unsafe.Add(ref _reference, start), length);
        }

        public ref T this[int index]
        {
            get
            {
                if ((uint)index >= (uint)_length)
                    throw new IndexOutOfRangeException();
                return ref Unsafe.Add(ref _reference, index);
            }
        }

        [MethodImpl(MethodImplOptions.InternalCall)]
        public extern ref T GetPinnableReference();
    }

    [Stub(InPlaceOf = "System.ReadOnlySpan`1")]
    public readonly ref struct ReadOnlySpan<T>
    {
        internal readonly ref T _reference;
        internal readonly int _length;

        public ReadOnlySpan(T[] array)
        {
            if (array == null)
            {
                _reference = ref Unsafe.NullRef<T>();
                _length = 0;
                return;
            }

            _length = array.Length;
            _reference = ref Internal.GetArrayData(array);
        }

        public ReadOnlySpan(T[] array, int start, int length)
        {
            if (array == null)
            {
                if (start != 0 || length != 0)
                    throw new NullReferenceException();
                _reference = ref Unsafe.NullRef<T>();
                _length = 0;
                return;
            }
             if ((uint)start > (uint)array.Length || (uint)length > (uint)(array.Length - start))
                throw new ArgumentOutOfRangeException();

            _length = length;
            _reference = ref Unsafe.Add(ref Internal.GetArrayData(array), start);
        }

        public ReadOnlySpan(ref T ptr, int length)
        {
            _reference = ref ptr;
            _length = length;
        }

        unsafe public ReadOnlySpan(void* ptr, int length)
        {
            _reference = ref Unsafe.AsRef<T>(ptr);
            _length = length;
        }

        public ReadOnlySpan(ref T ptr)
        {
            _reference = ref ptr;
            _length = 1;
        }

        public int Length => _length;

        public bool IsEmpty => _length == 0;

        public static ReadOnlySpan<T> Empty => default;

        public static implicit operator ReadOnlySpan<T>(T[] array) => new ReadOnlySpan<T>(array);

        public void CopyTo(Span<T> destination)
        {
             if ((uint)_length > (uint)destination.Length)
                 throw new ArgumentException();
             
             for (int i = 0; i < _length; i++)
             {
                 destination[i] = this[i];
             }
        }

        public ReadOnlySpan<T> Slice(int start)
        {
             if ((uint)start > (uint)_length)
                 throw new ArgumentOutOfRangeException();
             return new ReadOnlySpan<T>(ref Unsafe.Add(ref _reference, start), _length - start);
        }

        public ReadOnlySpan<T> Slice(int start, int length)
        {
             if ((uint)start > (uint)_length || (uint)length > (uint)(_length - start))
                 throw new ArgumentOutOfRangeException();
             return new ReadOnlySpan<T>(ref Unsafe.Add(ref _reference, start), length);
        }

        public ref readonly T this[int index]
        {
            get
            {
                 if ((uint)index >= (uint)_length)
                    throw new IndexOutOfRangeException();
                return ref Unsafe.Add(ref _reference, index);
            }
        }

        [MethodImpl(MethodImplOptions.InternalCall)]
        public extern ref readonly T GetPinnableReference();
    }
}
