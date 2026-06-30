using System.Collections;
using System.Collections.Generic;
using System.Runtime.CompilerServices;
using JetBrains.Annotations;

namespace DotnetRs;

[Stub(InPlaceOf = "System.Array")]
public class Array : ICloneable, IList, IStructuralComparable, IStructuralEquatable
{
    // Sentinel for array constructors used by runtime
    [UsedImplicitly] internal void CtorArraySentinel() { }

    public extern int Length { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public long LongLength => GetLongLength(0);

    public nuint NativeLength => (nuint)Length;

    public static int MaxLength => int.MaxValue;

    public extern int Rank { [MethodImpl(MethodImplOptions.InternalCall)] get; }

    public virtual int GetLength(int dimension)
    {
        if (dimension != 0) throw new IndexOutOfRangeException();
        return Length;
    }

    public long GetLongLength(int dimension) => GetLength(dimension);

    public virtual int GetRank() => Rank;

    public virtual int GetLowerBound(int dimension)
    {
        if (dimension != 0) throw new IndexOutOfRangeException();
        return 0;
    }

    public virtual int GetUpperBound(int dimension)
    {
        if (dimension != 0) throw new IndexOutOfRangeException();
        return Length - 1;
    }

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern object? GetValue(int index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern object? GetValue(long index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public virtual extern object? GetValue(params int[] indices);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern void SetValue(object? value, int index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public extern void SetValue(object? value, long index);

    [MethodImpl(MethodImplOptions.InternalCall)]
    public virtual extern void SetValue(object? value, params int[] indices);

    public object Clone() => MemberwiseClone();

    // IList implementation
    public bool IsFixedSize => true;
    public bool IsReadOnly => false;
    public bool IsSynchronized => false;
    public object SyncRoot => this;
    public int Count => Length;

    object? IList.this[int index]
    {
        get => GetValue(index);
        set => SetValue(value, index);
    }

    int IList.Add(object? value) => throw new NotSupportedException();
    void IList.Clear() => throw new NotSupportedException();
    bool IList.Contains(object? value) => IndexOf(this, value) >= 0;
    int IList.IndexOf(object? value) => IndexOf(this, value);
    void IList.Insert(int index, object? value) => throw new NotSupportedException();
    void IList.Remove(object? value) => throw new NotSupportedException();
    void IList.RemoveAt(int index) => throw new NotSupportedException();

    public IEnumerator GetEnumerator() => new ArrayEnumerator(this);

    public void CopyTo(System.Array array, int index)
    {
        ArgumentNullException.ThrowIfNull(array);
        if (Rank != 1) throw new RankException();
        ArgumentOutOfRangeException.ThrowIfNegative(index);
        if (index + Length > array.Length) throw new ArgumentException();

        for (int i = 0; i < Length; i++)
        {
            array.SetValue(GetValue(i), index + i);
        }
    }

    // IStructuralComparable / IStructuralEquatable stubs
    int IStructuralComparable.CompareTo(object? other, IComparer comparer) => throw new NotImplementedException();
    bool IStructuralEquatable.Equals(object? other, IEqualityComparer comparer) => throw new NotImplementedException();
    int IStructuralEquatable.GetHashCode(IEqualityComparer comparer) => throw new NotImplementedException();

    // Static helper methods
    public static int IndexOf(Array array, object? value)
    {
        ArgumentNullException.ThrowIfNull(array);
        return IndexOf(array, value, 0, array.Length);
    }

    public static int IndexOf(Array array, object? value, int startIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        return IndexOf(array, value, startIndex, array.Length - startIndex);
    }

    public static int IndexOf(Array array, object? value, int startIndex, int count)
    {
        ArgumentNullException.ThrowIfNull(array);
        if (startIndex < 0 || startIndex > array.Length) throw new ArgumentOutOfRangeException(nameof(startIndex));
        if (count < 0 || startIndex + count > array.Length) throw new ArgumentOutOfRangeException(nameof(count));

        for (int i = startIndex; i < startIndex + count; i++)
        {
            var item = array.GetValue(i);
            if (item == null && value == null) return i;
            if (item != null && item.Equals(value)) return i;
        }
        return -1;
    }

    public static int LastIndexOf(Array array, object? value)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf(array, value, array.Length - 1, array.Length);
    }

    public static int LastIndexOf(Array array, object? value, int startIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf(array, value, startIndex, startIndex + 1);
    }

    public static int LastIndexOf(Array array, object? value, int startIndex, int count)
    {
        ArgumentNullException.ThrowIfNull(array);
        if (array.Length == 0) return -1;
        if (startIndex < 0 || startIndex >= array.Length) throw new ArgumentOutOfRangeException(nameof(startIndex));
        if (count < 0 || startIndex - count + 1 < 0) throw new ArgumentOutOfRangeException(nameof(count));

        for (int i = startIndex; i > startIndex - count; i--)
        {
            var item = array.GetValue(i);
            if (item == null && value == null) return i;
            if (item != null && item.Equals(value)) return i;
        }
        return -1;
    }

    public static int IndexOf<T>(T[] array, T value)
    {
        return IndexOf((Array)(object)array, value, 0, array.Length);
    }

    public static int IndexOf<T>(T[] array, T value, int startIndex)
    {
        return IndexOf((Array)(object)array, value, startIndex, array.Length - startIndex);
    }

    public static int IndexOf<T>(T[] array, T value, int startIndex, int count)
    {
        return IndexOf((Array)(object)array, value, startIndex, count);
    }

    public static int LastIndexOf<T>(T[] array, T value)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf((Array)(object)array, value, array.Length - 1, array.Length);
    }

    public static int LastIndexOf<T>(T[] array, T value, int startIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        return LastIndexOf((Array)(object)array, value, startIndex, startIndex + 1);
    }

    public static int LastIndexOf<T>(T[] array, T value, int startIndex, int count)
    {
        return LastIndexOf((Array)(object)array, value, startIndex, count);
    }

    public static void Copy(Array sourceArray, Array destinationArray, int length)
    {
        Copy(sourceArray, 0, destinationArray, 0, length);
    }

    public static void Copy(Array sourceArray, int sourceIndex, Array destinationArray, int destinationIndex, int length)
    {
        ArgumentNullException.ThrowIfNull(sourceArray);
        ArgumentNullException.ThrowIfNull(destinationArray);
        ArgumentOutOfRangeException.ThrowIfNegative(length);
        ArgumentOutOfRangeException.ThrowIfNegative(sourceIndex);
        ArgumentOutOfRangeException.ThrowIfNegative(destinationIndex);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(sourceIndex + length, sourceArray.Length);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(destinationIndex + length, destinationArray.Length);

        for (int i = 0; i < length; i++)
        {
            destinationArray.SetValue(sourceArray.GetValue(sourceIndex + i), destinationIndex + i);
        }
    }

    public static void Clear(Array array)
    {
        ArgumentNullException.ThrowIfNull(array);
        Clear(array, 0, array.Length);
    }

    public static void Clear(Array array, int index, int length)
    {
        ArgumentNullException.ThrowIfNull(array);
        ArgumentOutOfRangeException.ThrowIfNegative(index);
        ArgumentOutOfRangeException.ThrowIfNegative(length);
        if (index + length > array.Length) throw new ArgumentException();

        for (int i = index; i < index + length; i++)
        {
            array.SetValue(null, i);
        }
    }

    public static T[] Empty<T>() => new T[0];

    public static void Resize<T>(ref T[]? array, int newSize)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(newSize);

        if (array == null)
        {
            array = new T[newSize];
            return;
        }

        if (array.Length == newSize)
        {
            return;
        }

        T[] newArray = new T[newSize];
        int copyLength = Math.Min(array.Length, newSize);
        Copy((Array) (object) array, 0, (Array) (object) newArray, 0, copyLength);
        array = newArray;
    }

    private class ArrayEnumerator(Array array) : IEnumerator
    {
        private int _index = -1;

        public object? Current => array.GetValue(_index);

        public bool MoveNext()
        {
            if (_index < array.Length - 1)
            {
                _index++;
                return true;
            }
            return false;
        }

        public void Reset() => _index = -1;
    }

}

internal sealed class SZArrayHelper<T> : IEnumerable<T>, ICollection<T>, IList<T>, IReadOnlyCollection<T>, IReadOnlyList<T>
{
    private static Array AsArray(object self) => (Array)self;

    public int Count => AsArray(this).Length;
    public bool IsReadOnly => true;

    public T this[int index]
    {
        get => (T)AsArray(this).GetValue(index)!;
        set => throw new NotSupportedException();
    }

    public void Add(T item) => throw new NotSupportedException();
    public void Clear() => throw new NotSupportedException();
    public bool Remove(T item) => throw new NotSupportedException();
    public void Insert(int index, T item) => throw new NotSupportedException();
    public void RemoveAt(int index) => throw new NotSupportedException();

    public bool Contains(T item) => Array.IndexOf(AsArray(this), item) >= 0;

    public int IndexOf(T item) => Array.IndexOf(AsArray(this), item);

    public void CopyTo(T[] array, int arrayIndex)
    {
        ArgumentNullException.ThrowIfNull(array);
        ArgumentOutOfRangeException.ThrowIfNegative(arrayIndex);

        var source = AsArray(this);
        if (arrayIndex + source.Length > array.Length) throw new ArgumentException();

        for (int i = 0; i < source.Length; i++)
        {
            array[arrayIndex + i] = (T)source.GetValue(i)!;
        }
    }

    public IEnumerator<T> GetEnumerator() => new SZGenericArrayEnumerator<T>(AsArray(this));

    IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();

    private sealed class SZGenericArrayEnumerator<TElement>(Array array) : IEnumerator<TElement>
    {
        private int _index = -1;

        object? IEnumerator.Current { get { return Current; } }
        public TElement Current { get { return (TElement)array.GetValue(_index)!; } }

        public bool MoveNext()
        {
            if (_index < array.Length - 1)
            {
                _index++;
                return true;
            }
            return false;
        }

        public void Reset() => _index = -1;
        public void Dispose() { }
    }
}
