#![cfg(feature = "async")]
use rtrb::RingBuffer;
use rtrb::chunks::AsyncChunkError;

#[tokio::test]
async fn exceed_capacity(){
    let (mut p, mut c) = RingBuffer::<i32>::new_async(1);
    assert!(matches!(p.write_chunk_async(2).await.unwrap_err(),AsyncChunkError::ExceedCapacity(1))); 
    assert!(matches!(c.read_chunk_async(2).await.unwrap_err(),AsyncChunkError::ExceedCapacity(1)));
}

#[tokio::test]
async fn read_async(){
    let (mut p, mut c) = RingBuffer::new_async(1);
    let value = 2021;
    let consume = tokio::spawn(async move{
        let chunk = c.read_chunk_async(1).await.unwrap();
        chunk.into_iter().next().unwrap()
    });
    p.push(value).unwrap();
    assert_eq!(consume.await.unwrap(),value);
    
}
#[tokio::test]
async fn write_async(){
    let (mut p, mut c) = RingBuffer::new_async(1);
    p.push(0).unwrap();
    let value = 2021;
    let produce = tokio::spawn(async move{
        let chunk = p.write_chunk_uninit_async(1).await.unwrap();
        chunk.fill_from_iter(core::iter::once(value));
    });

    assert_eq!(c.pop().unwrap(),0);
    produce.await.unwrap();
    assert_eq!(c.pop().unwrap(),value);
    
}
#[tokio::test]
async fn abandon(){
    let (p, mut c) = RingBuffer::<i32>::new_async(1);
    tokio::join!(async move{
        assert!(matches!(c.read_chunk_async(1).await.unwrap_err(),AsyncChunkError::TooFewSlotsAndAbandoned(0)));
        assert!(matches!(c.read_chunk_async(1).await.unwrap_err(),AsyncChunkError::TooFewSlotsAndAbandoned(0)));
    },async move{drop(p)});
}
#[tokio::test]
async fn deadlock(){
    let (mut p, mut c) = RingBuffer::<i32>::new_async(2);
    p.push(1).unwrap();
    tokio::join!(async move{
        c.read_chunk_async(2).await.unwrap();
    },async move{
        assert!(matches!(p.write_chunk_uninit_async(2).await.unwrap_err(),AsyncChunkError::WillDeadlock(1,1)));
        p.push(2).unwrap();
    });
}
#[tokio::test]
async fn pipeline_push_pop(){
    let range1 = 0..1000;
    let range2 = range1.clone();
    let range3 = range2.clone();
    let (mut p1,mut c1) = RingBuffer::new_async(1);
    let (mut p2,mut c2) = RingBuffer::new_async(1);
    tokio::join!(async move{
        for x1 in range1 {
            p1.push_async(x1).await.unwrap();
        }
    },async move{

        for x2 in range2{
            assert_eq!(c1.pop_async().await,Ok(x2));
            p2.push_async(x2).await.unwrap();
        }
        assert_eq!(c1.pop_async().await,Err(rtrb::AsyncPopError::EmptyAndAbandoned));
        
    },async move{
        for x3 in range3{
            assert_eq!(c2.pop_async().await,Ok(x3));
        }
        assert_eq!(c2.pop_async().await,Err(rtrb::AsyncPopError::EmptyAndAbandoned));
    });
}

#[tokio::test]
async fn pipeline_push_pop_threaded(){
    let range1 = 0..1000;
    let range2 = range1.clone();
    let range3 = range2.clone();
    let (mut p1,mut c1) = RingBuffer::new_async(1);
    let (mut p2,mut c2) = RingBuffer::new_async(1);
    let results = tokio::join!(tokio::spawn(async move{
        for x1 in range1 {
            p1.push_async(x1).await.unwrap();
        }
    }),tokio::spawn(async move{

        for x2 in range2{
            assert_eq!(c1.pop_async().await,Ok(x2));
            p2.push_async(x2).await.unwrap();
        }
        assert_eq!(c1.pop_async().await,Err(rtrb::AsyncPopError::EmptyAndAbandoned));
        
    }),tokio::spawn(async move{
        for x3 in range3{
            assert_eq!(c2.pop_async().await,Ok(x3));
        }
        assert_eq!(c2.pop_async().await,Err(rtrb::AsyncPopError::EmptyAndAbandoned));
    }));
    results.0.unwrap();
    results.1.unwrap();
    results.2.unwrap();
}

#[tokio::test]
async fn pipeline_chunks(){
    let capacity= 100;
    
    let elements = 1000;
    let (mut p1,mut c1) = RingBuffer::new_async(capacity);
    let (mut p2,mut c2) = RingBuffer::new_async(capacity);
    tokio::join!(async move{
        let mut x = 0;
        let chunk_size = 50;
        loop{
            let chunk = p1.write_chunk_uninit_async(chunk_size).await.unwrap();

            assert_eq!(chunk.fill_from_iter(x..x+chunk_size),chunk_size);
            x += chunk_size;
            match x.cmp(&elements){
                std::cmp::Ordering::Less => continue,
                std::cmp::Ordering::Equal => break,
                std::cmp::Ordering::Greater => unreachable!(),
            }
        }
        
    },async move{
        let chunk_size = 100;
        loop{
            if let Ok(read_chunk) = c1.read_chunk_async(chunk_size).await{
                let write_chunk = p2.write_chunk_uninit_async(chunk_size).await.unwrap();
                write_chunk.fill_from_iter(read_chunk);
            }else {
                break;
            }
        }
        
    },async move{
        let chunk_size = 25;
        let mut vec = Vec::new();
        loop{
            if let Ok(read_chunk) = c2.read_chunk_async(chunk_size).await{
                vec.extend(read_chunk);
            }else {
                break;
            }
        }
        for (i,x) in vec.into_iter().enumerate(){
            assert_eq!(x,i);
        }
    });
}

#[tokio::test]
async fn pipeline_chunks_threaded(){
    let capacity= 100;
    
    let elements = 1000;
    let (mut p1,mut c1) = RingBuffer::new_async(capacity);
    let (mut p2,mut c2) = RingBuffer::new_async(capacity);
    let results = tokio::join!(tokio::spawn(async move{
        let mut x = 0;
        let chunk_size = 50;
        loop{
            let chunk = p1.write_chunk_uninit_async(chunk_size).await.unwrap();

            assert_eq!(chunk.fill_from_iter(x..x+chunk_size),chunk_size);
            x += chunk_size;
            match x.cmp(&elements){
                std::cmp::Ordering::Less => continue,
                std::cmp::Ordering::Equal => break,
                std::cmp::Ordering::Greater => unreachable!(),
            }
        }
        
    }),tokio::spawn(async move{
        let chunk_size = 100;
        loop{
            if let Ok(read_chunk) = c1.read_chunk_async(chunk_size).await{
                let write_chunk = p2.write_chunk_uninit_async(chunk_size).await.unwrap();
                write_chunk.fill_from_iter(read_chunk);
            }else {
                break;
            }
        }
        
    }),tokio::spawn(async move{
        let chunk_size = 25;
        let mut vec = Vec::new();
        loop{
            if let Ok(read_chunk) = c2.read_chunk_async(chunk_size).await{
                vec.extend(read_chunk);
            }else {
                break;
            }
        }
        for (i,x) in vec.into_iter().enumerate(){
            assert_eq!(x,i);
        }
    }));
    results.0.unwrap();
    results.1.unwrap();
    results.2.unwrap();
}